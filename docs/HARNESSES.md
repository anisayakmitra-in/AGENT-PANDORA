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
- inline trust evidence;
- optional bounded Auto Route hints for a Domain Harness;
- composition metadata for a Meta Harness.

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

The package store does not load code, enable a Harness, issue a permit, or grant
runtime authority. A recorded package is metadata and verified bytes. A later
exact-version run may assemble an installed WebAssembly Gene through the
separate boundary documented in [WebAssembly package Genes](WASM.md).

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

Its built-in composition contains `coding-domain`, `research-domain`,
`design-domain`, `operations-domain`, `security-domain`, and
`debugging-domain`, and `data-domain`.

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
pandora registry set --name m-place --registry-url <url> [--token-env <name>]
pandora package install <id> [version]
pandora package list
pandora package inspect <id> <version>
pandora package enable <id> <version> --dry-run
pandora package enable <id> <version> --yes
pandora package disable <id> <version> --dry-run
pandora package disable <id> <version> --yes
pandora package rollback <id> --dry-run
pandora package rollback <id> --yes
pandora package lock
pandora package verify-lock
pandora package remove <id> <version> --dry-run
pandora package remove <id> <version> --yes
```

The native desktop Package Manager includes a Manifest Workbench for Domain,
Meta, and Gene envelopes. It produces a copyable, deterministic JSON preview
with the package kind, exact version, content hash, dependencies, runtime
compatibility, route hints or Meta composition, and `unverified` trust
evidence. The workbench is intentionally preview-only: it does not sign,
write files, admit records, enable bindings, publish packages, or handle
private keys. Users may pass the copied manifest through the existing local
`package admit` command, where the normal runtime validation and artifact hash
checks remain authoritative. Local signing is still a separate future surface
and must introduce an explicit key boundary before it is added.

`package admit` uses one local manifest as both the declared and embedded record.
Local hexadecimal trust evidence remains supported. `package install` consumes
current or exact-version M-Place metadata, requests the direct exact-version bytes
from the same configured registry, and retains the registry's base64 evidence
without re-encoding it. The client follows no redirects and does not request the
registry-controlled upstream `artifact_url`. Named registry profiles persist only
the validated base URL and an optional secret reference; they do not add authority,
change admission policy, or store a token in configuration.

Remote registry admission is Gene-only in this release. It requires an artifact,
a canonical lowercase SHA-256 digest, no unresolved capability requirements,
and one valid Pandora runtime requirement. Other recognized registry kinds fail
before download or durable state change. Pinned GitHub and local admission use
the same package checks and may admit the Domain and Meta profile kinds
described below.

The signed payload starts with `pandora-package-signature-v2` and binds the
package ID, version, kind, publisher, artifact hash, runtime compatibility,
license, exact dependencies, Meta composition, and Domain route hints through a
deterministic length-prefixed encoding. Trust evidence is excluded from the
payload because it contains the signature itself. Changing a route, dependency,
composition member, package kind, or artifact invalidates the signature.

A valid package signature proves that the declared public key signed those
exact fields. It does not establish publisher trust. Local `official` claims
remain rejected until a publisher root is configured. Registry moderation
levels are not promoted into local `Official` trust. Admission still records
metadata only: it does not load code, enable a Harness, issue a permit, or grant
runtime authority.

Every admitted package starts disabled. `package enable` is the separate local
lifecycle boundary: it requires an exact ID and version, verifies that required
package dependencies and composed custom Domain Harnesses are already enabled,
and binds at most one active version for a package ID. Switching to another
admitted version retains the former exact version as a one-step rollback target.
`package disable` keeps the verified bytes but removes them from runtime and
slash-command selection. Enable, disable, and rollback all support `--dry-run`;
mutations require `--yes` and refuse to break an enabled dependent. These
bindings do not grant effect authority and cannot alter Parliament, Shadow
Council, ReferenceMonitor, permits, or the constitutional service.

Multiple exact versions may coexist so an update can be staged before its
binding changes. An active version cannot be removed. A retained rollback
version cannot be removed while another version is active; once a package is
disabled, confirmed removal clears its inactive rollback marker together with
the package record.
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
authority. A custom profile becomes selectable only after its exact package
version is enabled, and every run still passes through the governed execution
path.

Meta Harnesses coordinate existing Domain Harnesses. They do not augment a
constitutional service, execute effects, install packages, or grant permits.

## Coding Domain Harness

The built-in `coding-domain` Harness owns eighteen Genes. Thirteen are narrow execution
primitives:

- `workspace.read` reads one scoped file;
- `workspace.search` searches bounded regular files;
- `patch.apply` writes one scoped file after approval;
- `verification.run` runs the fixed verifier after approval;
- `tests.run` runs the fixed test command after approval;
- `format.check` runs the fixed formatter check after approval;
- `lint.check` runs the fixed workspace lint check after approval;
- `build.check` runs the locked workspace build after approval;
- `workspace.status` reads the short Git status for the workspace;
- `workspace.diff` reads the bounded Git diff for the workspace;
- `workspace.log` reads the twenty most recent Git commits for the workspace;
- `workspace.refs` lists the fifty most recent branches, remotes, and tags;
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

## Design Domain Harness

The built-in `design-domain` Harness owns six Genes:

- `design.inventory` lists bounded workspace files;
- `design.tokens` searches fixed CSS and theme-token markers;
- `design.inspect` reads one scoped design source;
- `design.compare` reads two distinct scoped design sources and labels both;
- `accessibility.evidence` searches fixed semantic markup markers without
  claiming conformance;
- `design.guide` returns static usage guidance and requests no effect.

The effectful Design Genes request only `filesystem.read`. They use the same
execution profile, Parliament decision, Reference Monitor permit, filesystem
executor, receipts, and runtime events as every other built-in Gene. This
release does not give the Design Domain browser control, image generation,
network access, rendering, or automated standards certification.

## Operations Domain Harness

The built-in `operations-domain` Harness owns six Genes:

- `operations.inventory` lists bounded workspace files;
- `operations.search` searches bounded local evidence;
- `config.inspect` reads one scoped configuration source;
- `config.compare` reads two distinct scoped sources and labels both results;
- `deployment.evidence` searches fixed container, Compose, Kubernetes, and
  workflow markers without claiming deployability;
- `operations.guide` returns static usage guidance and requests no effect.

The effectful Operations Genes request only `filesystem.read`. They cannot run
commands, connect to infrastructure, read credentials outside the workspace, or
change a deployment. Those effects require separate capabilities and approvals.

## Security Domain Harness

The built-in `security-domain` Harness is Pandora's staged, evidence-first
workflow surface. It owns eighteen read-only evidence Genes:

- `security.assess` performs one bounded fixed-marker evidence pass without
  claiming complete scanner coverage;
- `security.scan` inventories fixed security-boundary markers;
- `security.deep-scan` searches a broader fixed marker set without claiming
  complete scanner coverage;
- `security.diff-scan` searches changed-code and regression terminology without
  reviewing a specific revision;
- `security.audit` searches fixed high-signal source markers such as `unsafe`,
  process spawning, network clients, deserialization, and secret terminology;
- `security.dependencies` searches fixed dependency declaration markers;
- `security.threat-model` searches trust-boundary, attacker, sandbox, and
  isolation terminology;
- `security.discovery` records candidate source, control, sink, and reachability
  terminology without asserting a finding;
- `security.triage` searches existing finding, vulnerability, advisory, and
  proof terminology without assigning a verdict;
- `security.attack-path` searches source, control, sink, impact, and privilege
  evidence without proving exploitability;
- `security.validation` searches test and validation evidence without running
  a scanner;
- `security.fix` searches remediation planning terminology without changing
  code;
- `security.verify-fix` searches regression and negative-control evidence
  without certifying a fix;
- `security.writeup` searches disclosure fields without generating a
  vulnerability report;
- `security.track` searches finding lifecycle fields without creating or
  mutating a finding record;
- `security.hardening` searches local defensive-control evidence without
  changing code;
- `security.policy` searches fixed authorization, credential, and security-policy
  markers;
- `security.guide` returns static guidance without requesting an effect.

The Security Domain follows a staged lifecycle: standard, deep, diff,
threat-model, discovery, triage, attack-path, validation, fix, verification,
writeup, tracking, hardening, and policy workflows. Every stage is a native
Pandora workflow.

It remains a bounded assessment surface, not a complete vulnerability scanner,
finding database, or compliance certification. It does not execute scanners,
run commands, contact networks, assign triage verdicts, modify files, inspect
credential values, or remediate findings. Effectful Genes use the existing
workspace-scoped `filesystem.read` permit, receipt, and runtime-event path.
Process-backed validation and remediation require separate capabilities and
approvals.

## Debugging Domain Harness

The built-in `debugging-domain` Harness owns six read-only evidence Genes:

- `debugging.inventory` lists bounded workspace files;
- `debugging.failures` searches fixed crash and error markers;
- `debugging.tests` searches fixed test and assertion markers;
- `debugging.regressions` searches reproduction and comparison markers;
- `debugging.diagnostics` searches fixed runtime symptom markers;
- `debugging.guide` returns static workflow guidance without requesting an effect.

The Debugging Domain records evidence for investigation. It does not run tests,
execute a debugger, change files, infer a root cause, or claim that a marker is
a defect. Process execution and code changes remain separate governed actions.

## Data Domain Harness

The built-in `data-domain` Harness owns six bounded evidence Genes:

- `data.inventory` lists bounded workspace files;
- `data.schema` searches fixed schema and data-model markers;
- `data.quality` searches fixed validation and integrity markers;
- `data.lineage` searches source, transformation, pipeline, and provenance markers;
- `data.analysis` searches fixed statistical and aggregation markers;
- `data.guide` returns static workflow guidance without requesting an effect.

The Data Domain records local evidence without connecting to databases, executing
queries, contacting networks, changing data, or claiming statistical correctness.
Database, network, process, and mutation actions remain separate governed
capabilities.

`CodingFeedbackLoop` composes the existing evaluation, Reflexion, adaptation,
and run-loop contracts around coding evidence. A verified iteration completes
without adaptation. A failed retryable iteration records trajectory, outcome,
and policy results, creates a redacted Reflexion artifact, and selects only an
approved candidate that remains within the adaptation policy. Non-retryable or
budget-exhausted work stops without selecting another strategy.

The feedback loop evaluates evidence; it does not call a model, run a Gene,
mint a permit, mutate code, or change policy. The CLI starts it only when a
direct `pandora run` supplies `--expected-output`; agent-mode runs keep their
existing path. The observed output must be bounded UTF-8 text, and one trailing
line ending is ignored. A retry remains a recommendation for the normal
governed run path, not an automatic second execution.

`SelfHealingEngine` is the bounded recovery selector used when a feedback loop
offers recovery or capability-reduction candidates. It reuses the existing
adaptation policy and receipt, ignores ordinary workflow/provider candidates,
and never executes a recovery action or expands authority. The run loop remains
responsible for retry and termination budgets; the caller must send any chosen
recovery through the normal governed execution path.

Use `pandora feedback coding` to evaluate one already-redacted coding iteration
without executing it or changing durable state:

```text
pandora feedback coding --session <id> --execution <id> \
  --request-digest <digest> --expected-output "tests passed" \
  --output "tests passed" --retryable --json
```

The command returns trajectory, outcome, and policy evaluations, a bounded
Reflexion artifact on failure, and the fixed `coding.safe_retry` recovery
candidate when `--retryable` is supplied. Its report is intentionally
`report-only`; use the normal governed run path for any subsequent retry.

Direct runs can opt into the same feedback loop after execution:

```text
pandora run "read:README.md" --expected-output "fixture" --retryable --json
```

The direct-run result includes `coding_feedback` and records its evaluation
through the existing session and rollout store. `--retryable` requires
`--expected-output`; these options are not available with `--agent`.

Every built-in Harness and Gene has a canonical slash command. The Coding short
aliases are `/coding`, `/read`, `/search`, `/patch`, `/verify`, `/test`, `/format`, `/lint`, `/build`, `/status`, `/diff`, `/log`, `/refs`, `/review`,
`/audit`, `/argus-review`, `/debt`, `/measure`, and `/guide`. Canonical commands
encode their identities, for example `/harness:coding-domain` and
`/gene:coding-domain:daedalus.audit`.

The Research short aliases are `/research`, `/evidence-inventory`,
`/evidence-search`, `/source-read`, `/source-compare`, `/citation-inventory`,
and `/research-guide`.

The Design short aliases are `/design`, `/design-inventory`, `/design-tokens`,
`/design-inspect`, `/design-compare`, `/accessibility-evidence`, and
`/design-guide`.

The Operations short aliases are `/operations`, `/operations-inventory`,
`/operations-search`, `/config-inspect`, `/config-compare`,
`/deployment-evidence`, and `/operations-guide`.

The Security short aliases are `/security`, `/security-audit`,
`/security-scan`, `/security-deep-scan`, `/security-diff-scan`,
`/security-dependencies`, `/security-threat-model`,
`/security-discovery`, `/security-triage`, `/security-attack-path`,
`/security-validation`, `/security-fix`, `/security-verify-fix`,
`/security-writeup`, `/security-track`, `/security-hardening`,
`/security-policy`, and `/security-guide`.

The Debugging short aliases are `/debugging`, `/debugging-inventory`,
`/debugging-failures`, `/debugging-tests`, `/debugging-regressions`,
`/debugging-diagnostics`, and `/debugging-guide`.

Admitted custom Domain and Meta profiles receive exact-version Harness commands.
Custom Domain profiles receive Gene commands for dependencies that resolve to
an available built-in Gene or an installed WebAssembly Gene at the declared
version. Their commands are namespaced, such as
`/gene:owner%2Fdomain@1.0.0:workspace.read`, and cannot replace built-in aliases.

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

`gene` packages can be installed as verified local records. A Gene artifact may
run only as import-free WebAssembly through an exact admitted Domain Harness
dependency, explicit policy approval, a one-shot permit, and the bounded
interpreter. Native package code is not supported. `meta_harness` packages can pass
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

The constitutional `core-source` ID is reserved. The optional built-in Domain
IDs and `coordination-meta` may be replaced by an exact package of the same
kind only when its trust level is `verified` and its Ed25519 signature passes.
Unsigned packages cannot replace a built-in entry. The replaceable Domain IDs
are `coding-domain`, `research-domain`, `design-domain`,
`operations-domain`, `security-domain`, `debugging-domain`, and
`data-domain`.

Replacement changes one entry in the active catalog snapshot; it does not add a
second entry with the same ID. Disabling the package restores the compiled
built-in on the next snapshot. Parliament, Shadow Council, ReferenceMonitor,
the constitutional Source binding, and permit issuance are not replacement
targets.

The built-in Coding, Research, Design, Operations, Security, Debugging, and Data
Genes and installed WebAssembly Genes are the executable Gene implementations available to
declarative Domain profiles. An admitted custom Domain profile can be selected
with its exact version when every required dependency resolves exactly. The
profile still uses the existing execution controller and effect policy. The
profile artifact is never loaded as code, and the profile cannot issue permits
or grant runtime authority.

## Local service activation snapshot

The local desktop service builds its runtime catalog from the package store's
active exact-version bindings at startup. Enabled custom Domain Harnesses load
before enabled Meta Harnesses, and their installed WebAssembly Gene
dependencies are resolved through the active artifact catalog. A Gene shared by
multiple Domain Harnesses is compiled once only when every Harness resolves the
same exact identity to the same artifact. Conflicting resolutions fail service
startup rather than selecting one implicitly.

The service exposes loaded custom Harnesses through `runtime.capabilities`, so
the desktop Harness Lab, Command selector, and saved workflows use the same
catalog that can execute. Enabling, disabling, updating, or rolling back a
package changes the persisted binding but does not mutate a running controller;
the desktop therefore requires a local-service restart before the new snapshot
is active.

An enabled custom Domain may include a `domain_routing.hints` list with 1-32
unique canonical strings, each 3-64 bytes. Auto Route compares those declared
hints with the lowercase task summary and selects the longest match across the
active custom and built-in routes. Two different Harnesses with the same top
score produce an ambiguity error and require explicit selection. A custom
Domain without route hints is explicit-selection only.

Image, video, VLSI, EDA, or another specialist Domain therefore needs no
compiled Pandora category. Its package can declare phrases such as "image
generation," "video generation," "vlsi design," or "verilog." Route metadata
selects only a Harness. It cannot select a Gene, enable a package, grant a
capability, approve an effect, or issue a permit. An explicit user Harness
selection always takes precedence.

In agent mode, each loaded WebAssembly Gene receives a deterministic,
provider-safe tool alias. That alias is presentation only: invocation binds
back to the exact admitted Gene ID inside the selected Harness. Shared exact
Gene identities are exposed once, while conflicting versions fail agent startup
instead of offering an ambiguous tool.

Loading a package never changes the constitutional path. A WebAssembly Gene
still receives a bound execution profile, policy evaluation, exact approval
when required, a fresh one-shot ReferenceMonitor permit, and an effect receipt.
Parliament, Shadow Council, ReferenceMonitor, and the permit system are not
package bindings and cannot be replaced through this catalog.

## Ownership

The package envelope is owned by `pandora-types`. Existing Harness and Gene manifests remain implementation contracts; `PackageManifest::from_harness` is the explicit adapter between them.

M-Place remains a separate registry. It does not own activation, permissions,
runtime events, or execution policy.
