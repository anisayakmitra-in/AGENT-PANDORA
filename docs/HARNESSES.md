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

Package and dependency versions use strict SemVer, including prerelease and
build metadata. The exact version is part of package identity; `1.0` and
label-style versions are rejected.

Harnesses, Genes, and Source service bindings use the same exact SemVer rule.
A Harness cannot declare the same owned Gene twice, so its ownership list is
an unambiguous execution boundary.

Runtime compatibility uses a non-wildcard `pandora` SemVer requirement, such
as `pandora>=2.0.0-alpha.0, <3.0.0`. Admission checks it against the running
Pandora version. SemVer prerelease rules apply: `pandora>=2.0.0` requires the
stable release and does not admit a package into the `2.0.0-alpha` line.

`pandora-runtime::PackageStore` compares the declared package metadata with the embedded manifest, hashes the supplied bytes, checks required dependencies, and records the verified metadata in the local `packages.sqlite3` store. The store retains the artifact bytes so a later process can revalidate the record before using it.

The package store does not load code, enable a Harness, issue a permit, or grant runtime authority. A recorded package is metadata that passed admission, not an executable extension.

The built-in `core-source` Harness is the runtime's Source Harness. It binds the
`pandora-runtime` constitutional service and its exact implementation version,
and owns no Genes. `harness list` and `harness inspect` expose both values as
`constitutional_service` and `constitutional_service_version`. It is
discoverable and inspectable, but it is not a user-runnable task target. Source
Harnesses augment one constitutional service; they do not provide a second
execution hierarchy.

`MemoryEngine`, `ContextEngine`, and `ObservabilityEngine` are internal runtime
engines in this release, not separate Source Harnesses. Source Harnesses are
core-owned; local package admission rejects `source_harness` records. A core
Source addition must bind one named service and pass its admission and approval
boundary before it can augment the runtime.

The built-in `coordination-meta` Meta Harness declares the Domain Harnesses it
may coordinate and its handoff ceiling. It owns no Genes. The orchestration
engine checks those limits before registering a plan, so a plan cannot introduce
an undeclared Domain Harness through a Meta Harness.

Its built-in composition contains `coding-domain` and `research-domain`.

`pandora harness inspect coordination-meta --json` exposes that composition
boundary as `meta_composition.allowed_domains` and
`meta_composition.max_handoffs`. This is metadata for routing and validation;
it is not a permission grant.

Harness inspection also reports the execution boundary explicitly. Source
Harnesses use `system_augmentation`, Meta Harnesses use `composition_only`, and
Domain Harnesses use `domain_execution`. Only a Domain Harness with registered
Genes is runnable; the other two kinds remain inspectable but are not task
targets.

Local package admission is explicit:

```text
pandora package admit --manifest <manifest.json> --artifact <artifact>
pandora package install <id> [version] --registry <url>
pandora package list
pandora package inspect <id> <version>
pandora package lock
pandora package verify-lock
pandora package remove <id> <version> --dry-run
pandora package remove <id> <version> --yes
```

`package admit` uses one local manifest as both the declared and embedded record.
Local hexadecimal trust evidence remains supported. `package install` consumes
current or exact-version M-Place metadata, requests the direct exact-version bytes
from the same configured registry, and retains the registry's base64 evidence
without re-encoding it. The client follows no redirects and does not request the
registry-controlled upstream `artifact_url`.

Remote admission is Gene-only in this release. It requires an artifact, a canonical
lowercase SHA-256 digest, no unresolved capability requirements, and one valid
Pandora runtime requirement. Other recognized registry kinds fail before download
or durable state change. The signed message is
`{id}:{version}:{publisher}:{content_hash}`, using the exact manifest strings.
This proves that the evidence matches the declared package and artifact; it
does not establish publisher trust. Local `official` claims remain rejected
until a publisher trust root is configured. Registry moderation levels are not
promoted into local `Official` trust. Admission still records metadata only: it
does not load code, enable a Harness, issue a permit, or grant runtime
authority.
Removal uses the exact package ID and version. A dry run changes nothing;
confirmed removal is transactional and refuses to remove a package required by
another admitted package or named by an admitted Meta Harness composition.
Optional dependencies do not block removal.

`package lock` writes a deterministic snapshot of every revalidated local record
to `<workspace>/pandora.lock`. Each entry retains the canonical manifest, exact
version, verified artifact hash, dependencies, compatibility, license, and inline
trust evidence. `package verify-lock` rejects malformed, oversized, noncanonical,
or stale locks. Writing the lockfile uses an atomic replacement.

`pandora harness list --json` keeps built-in Harnesses under `harnesses`,
reports the admitted Domain and Meta subset under `admitted_profiles`, and keeps
all local package records under `package_records`. Admitted profiles are
discoverable metadata; discovery does not enable them or grant runtime
authority. A Domain profile remains selectable only through an explicit,
exact-version governed run.

Meta Harnesses coordinate existing Domain Harnesses. They do not augment a
constitutional service, execute effects, install packages, or grant permits.

## Coding Domain Harness

The built-in `coding-domain` Harness owns ten Genes. Five are narrow execution
primitives:

- `workspace.read` reads one scoped file;
- `workspace.search` searches bounded regular files;
- `patch.apply` writes one scoped file after approval;
- `verification.run` runs the fixed verifier after approval;
- `change.review` reads one file for review.

Five are bounded coding workflows:

- `daedalus.audit` inventories the workspace without following symlinks;
- `argus.review` reads one named file for focused review;
- `ariadne.debt` searches only the four fixed debt markers defined by the runtime;
- `hephaestus.measure` runs only `cargo check --locked`;
- `athena.guide` returns static command guidance and requests no effect.

Each effectful workflow creates the same typed operation requests as the narrow
Genes. Parliament decides policy, the Reference Monitor issues a one-shot
permit, and the executor records the result. A workflow cannot bypass that
path.

## Research Domain Harness

The built-in `research-domain` Harness owns six Genes:

- `evidence.inventory` lists bounded workspace files;
- `evidence.search` searches bounded workspace evidence;
- `source.read` reads one scoped source;
- `source.compare` reads two distinct scoped sources and labels both results;
- `citation.inventory` searches the fixed `http://`, `https://`, and `doi:`
  markers without claiming that a citation is valid;
- `research.guide` returns static usage guidance and requests no effect.

The effectful Research Genes request only `filesystem.read`. They use the same
execution profile, Parliament decision, Reference Monitor permit, filesystem
executor, receipts, and runtime events as Coding Genes. They do not grant
network access. External retrieval requires a separately configured governed
tool or MCP server.

`CodingFeedbackLoop` composes the existing evaluation, Reflexion, adaptation,
and run-loop contracts around coding evidence. A verified iteration completes
without adaptation. A failed retryable iteration records trajectory, outcome,
and policy results, creates a redacted Reflexion artifact, and selects only an
approved candidate that remains within the adaptation policy. Non-retryable or
budget-exhausted work stops without selecting another strategy.

The feedback loop evaluates evidence; it does not call a model, run a Gene,
mint a permit, mutate code, or change policy. The current CLI does not start
this loop automatically. A caller must supply the expected outcome, usage,
retry classification, and approved adaptation candidates.

Every built-in Harness and Gene has a canonical slash command. The Coding short
aliases are `/coding`, `/read`, `/search`, `/patch`, `/verify`, `/review`,
`/audit`, `/argus-review`, `/debt`, `/measure`, and `/guide`. Canonical commands
encode their identities, for example `/harness:coding-domain` and
`/gene:coding-domain:daedalus.audit`.

The Research short aliases are `/research`, `/evidence-inventory`,
`/evidence-search`, `/source-read`, `/source-compare`, `/citation-inventory`,
and `/research-guide`.

Admitted custom Domain and Meta profiles receive exact-version Harness commands.
Custom Domain profiles receive Gene commands only for dependencies that resolve
to available built-in Genes at the declared version. Their commands are
namespaced, such as `/gene:owner%2Fdomain@1.0.0:workspace.read`, and cannot
replace built-in aliases.

## Domain profiles

`DomainAgent` is a runtime profile of a Domain Harness, not a fourth Harness
kind. The runtime represents it as a `DomainAgentProfile` over one
`OrchestrationPlan` and `RunLoopConfig`. Registration requires every role to
belong to the same Domain Harness and keeps the existing Gene and effect
policy. Provider selection remains a provider-runtime concern and is not
embedded in the Harness or package contract.

A `Swarm` is the `DomainProfileMode::Swarm` form of that same runtime profile.
It records a bounded worker count within the plan's parallelism limit. A Swarm
does not create a parallel execution hierarchy. Cross-domain work still goes
through a Meta Harness and its declared composition.

## Package kinds

The external vocabulary is closed and uses these exact values:

`gene`, `domain_harness`, `meta_harness`, `source_harness`, `package`, `provider`, and `skill`.

`gene` packages can be admitted as verified local records, but they do not load
third-party executable code in this release. `meta_harness` packages can pass
the separate composition-profile admission boundary described below.
`domain_harness` packages can pass a profile-only admission boundary when they
declare at least one required dependency that resolves to an available Gene,
either from the local package store or the built-in catalog.
Source, Provider, Skill, and generic Package records remain recognized but
rejected until their lifecycles have their own validation and execution rules.

Custom Meta Harness packages may be admitted as composition-only metadata
profiles. Admission verifies their declared Domain members, exact artifact
hash, package identity, and required dependencies. Admission records use the
`admitted` state and never activate a Harness, execute native code, issue a
permit, or grant runtime authority.

Meta composition names Domain Harnesses by ID. A custom member must therefore
resolve to exactly one admitted Domain Harness profile, or to a built-in Domain
Harness. Missing members, non-Domain members, and multiple admitted versions
of the same custom ID are rejected during admission.

Built-in Harness IDs are reserved. A package can add a new Domain or Meta
profile but cannot shadow `core-source`, `coordination-meta`, `coding-domain`,
`research-domain`, or another built-in identity.

The built-in Coding and Research Genes are the executable Gene implementations
available to declarative Domain profiles. An admitted custom Domain profile can
be selected with its exact version when every required dependency maps to one
of those built-in Genes. The profile still uses the existing execution
controller and effect policy. Its artifact is never loaded as code, and the
profile cannot issue permits or grant runtime authority.

## Ownership

The package envelope is owned by `pandora-types`. Existing Harness and Gene manifests remain implementation contracts; `PackageManifest::from_harness` is the explicit adapter between them.

K-O Palace remains a separate registry. Pandora may consume verified package metadata later, but Palace does not own activation, permissions, runtime events, or execution policy.
