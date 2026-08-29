# Pandora roadmap

Pandora is roughly 60-70% complete against the local agent platform described
in this roadmap. The center of that range is about 65%. This is a planning
estimate from the source tree, tests, desktop, workflows, and documented release
gates on 2026-08-28. It is not a release claim.

The execution and authority core is ahead of the product loops around it.
Parliament, Shadow Council, ReferenceMonitor, exact Harness and Gene bindings,
one-shot permits, receipts, package admission, and governed replacement already
exist. The largest unfinished areas are automatic evaluation loops,
cross-session memory policy, hostile-input testing, publisher trust, fleet
operations, and signed native releases.

## Status by phase

| Phase | Estimate | Shipped in the source tree | Work still open |
| --- | ---: | --- | --- |
| 6. Production readiness | 85-90% | scoped identity, automatic local device trust, encrypted secrets, local telemetry and crash records, encrypted backup and restore, fresh-runner install/update/rollback/backup/restore/uninstall drills, update channels, release workflows, checksum signature verification, release evidence index, CodeQL, dependency audits | signed stable artifacts, real clean-machine release proof on every advertised platform, installer rollback exercises with the stable artifact |
| 7. Runtime scale and orchestration | 85-90% | persistent prompt-context cache, headless jobs, bounded parallel subagents, exact-commit worktrees, durable orchestration claims and receipts, interruption and resume rules, multi-repository plans, fleet leases, budgets, execution-bound lease renewal, and durable supervisor state with PID-bound worker heartbeats, process-wide execution leases for headless jobs and subagents, lease gating, stale-supervisor reconciliation without replay, bounded stale reaping, atomic PID-bound restart handoff, atomic cross-process quiescence guards, bounded independently launched job watch windows, long-lived local daemon workers with explicit drain/stop protocol, cross-process crash reconciliation/restart evidence, and bounded staggered-producer soak coverage | long-duration load and soak tests, expanded cancellation races, multi-repository partial failure |
| 8. Agent experience and disclosure | 75-85% | native desktop source, Command and Council inspection, background runs, runtime inventory, Harness Lab, package lifecycle, package manifest workbench, BYOK providers and models, MCP configuration, pinned GitHub packages, active custom Domain and Meta Harnesses, WebAssembly Genes, custom Auto Route contracts, optional built-in Domain and Meta replacement | local signing support with an explicit key boundary, broader Skill and provider package lifecycles, desktop accessibility pass, native installer release proof |
| 9. Evaluation-driven loops | 50-60% | trajectory, outcome, policy, regression, adversarial, golden, and holdout evaluation; coding feedback; research-only mutation and population strategies; durable evolution state; canary activation and rollback | scheduled evaluation loops, self-healing test generation, automatic canary policy, operator scorecards, quality gates for every artifact class |
| 10. Memory consolidation | 50-60% | scoped L0, L1, and L2 records; durable recall; approval-gated promotion; revocation, audit, and compaction; deterministic evidence-bound synthesis | cross-session and cross-project consolidation policy, scheduled synthesis, desktop removal and provenance views, source graph for consolidated lessons |
| 11. Adversarial resilience | 45-55% | path confinement, symlink checks, secret redaction, replay protection, exact signatures and hashes, fail-closed package and permit checks, adversarial evaluation primitives | tool-poisoning detection and quarantine, prompt-injection corpus across every input source, publisher trust roots and revocation, fuzzing and hostile multi-agent handoff suites |
| 12. Agent operations | 45-55% | three-platform CI, desktop CI, release and security workflows, bounded agent workers, orchestration receipts, local fleet records | agent CI/CD as a Pandora workflow, supervisor controls, fleet dashboards, multi-repository budget enforcement, stable channel promotion with real signing credentials |

The ranges separate code presence from operating proof. A component can be
implemented and tested locally while its clean-machine release, failure
recovery, or sustained-load evidence is still missing.

## Phase 6: close the release boundary

The local security controls exist. Phase 6 closes when a tagged native release
passes the documented gates on Windows, macOS, and Linux with no manual
exceptions beyond the recorded Linux GTK dependency advisory.

Next work:

- run clean-machine install, update, rollback, backup, restore, and uninstall
  drills on all advertised platforms;
- exercise real Windows signing and Apple signing and notarization in the stable
  workflow;
- test recovery after interruption during update, restore, and catalog
  activation;
- review the generated release evidence index before publishing and retain
  it with the release record.

## Phase 7: finish worker operations

Pandora can execute concurrent local subagents and persist orchestration state.
Subagent cancellation is now terminally race-safe: once a running cancellation
request is recorded, a late provider response cannot be stored as success. A
repeated two-connection SQLite race regression also proves cancellation and
finish always produce one durable terminal winner.
The durable queue also has an 8-worker/64-job claim-pressure regression proving
that each queued job is claimed once across separate SQLite connections. It
still needs the operating layer that keeps those workers healthy for days, not
one command invocation. Operators can now reap all heartbeat-stale supervisors in one bounded pass and perform an atomic PID-bound restart handoff. An independently launched `job work --watch --idle-timeout <1-3600>` window now binds its own PID, heartbeat, and execution lease, exits deterministically on idle timeout, external drain, or `--max-jobs`, and leaves durable stopped state. `job work --daemon` now keeps the same bounded authority and liveness records alive until an optional job cap or `pandora fleet supervisor drain job-worker` requests graceful stop. A bounded staggered-producer soak regression completes 16 jobs exactly once while the daemon is live. A killed worker can be reconciled and rebound to a new PID without replaying a claimed effect.

Next work:

The partial multi-repository failure regression now runs submit, claim,
completion, interruption, inspection, and resume in separate CLI processes.
It preserves the completed planner receipt and active maker role, and keeps
resume blocked until reconciliation.

- extend bounded soak coverage into long-duration load and recovery runs;
- expand cancellation-race coverage around provider return and worker shutdown;
- extend queue-pressure coverage into sustained soak runs;
- test worker crashes and partial multi-repository failure across independently
  restarted worker processes, including the reconciliation path.

## Phase 8: finish the modular product surface

An enabled Domain Harness may declare 1-32 canonical route hints. Verified package signatures bind the exact list. Auto Route reads
the active catalog, scores the longest matching hint, and fails closed when two
Harnesses tie. A Domain without route hints remains explicit-selection only.
Explicit user selection always wins. Route metadata can select a Harness but
cannot select a Gene, add a capability, approve an effect, or issue a permit.

This keeps image generation, video generation, VLSI, EDA, science, finance, or
any later Domain outside Pandora's compiled router. A package can declare terms
such as "image generation", "video generation", "vlsi design", or "verilog";
the runtime does not need a new hard-coded category.

Optional built-in Domain IDs and coordination-meta may be replaced only by an
exact enabled package with a valid verified signature and matching kind.
core-source, Parliament, Shadow Council, ReferenceMonitor, and the permit path
remain immutable. Disable or roll back the package to restore the compiled
entry on the next runtime snapshot.

The desktop package manager now includes a manifest workbench. It previews the
closed package vocabulary, exact JSON shape, bounded Domain route hints, Meta
composition, dependency declarations, and unverified trust posture. Copying
the JSON is the only mutation it performs; it does not sign, admit, enable,
publish, store private keys, or grant authority. The existing admission and
lifecycle boundaries remain the only path to durable package state.

It also previews exact route-hint overlaps across the local catalog before a
Domain is enabled. This is advisory evidence only; runtime routing still uses
active admitted bindings, explicit user selection wins, and ambiguous ties
fail closed.

Next work:

- add local signing support only with an explicit, non-exporting key boundary;
- complete keyboard, screen-reader, reduced-motion, scaling, and high-contrast
  checks in the native desktop;
- define separate admission rules before Provider, Skill, Source, or generic
  packages can become active.

## Phase 9: connect evidence into repeatable loops

The evaluator and governed activation lifecycle exist, but operators still
drive most transitions. Phase 9 makes those loops repeatable without letting
evaluation approve its own candidate.

Next work:

- schedule bounded prompt, Skill, workflow, and WebAssembly Gene evaluations;
- generate regression tests from verified failures and require review before
  they enter a suite;
- record cost, latency, stability, and quality scorecards per candidate;
- run canaries against fixed budgets and pause for Parliament approval before
  activation;
- expose every rejection, retry, and rollback as durable evidence.

## Phase 10: make memory useful across work

Pandora already stores scoped summaries and can synthesize an evidence-bound L1
record. It does not yet decide when lessons may cross a session or project.

Next work:

- define explicit consolidation scopes and conflict rules;
- add scheduled synthesis with stale-evidence checks;
- expose source records, promotion approval, revocation, and compaction in the
  desktop;
- add retention controls and secure-erasure guidance for databases, backups,
  and storage snapshots;
- connect evolution lineage queries to the exact memory evidence IDs that
  shaped a candidate.

## Phase 11: treat content as hostile

Pandora already confines effects and validates identity. Phase 11 adds defenses
against validly delivered but malicious content.

Next work:

- classify tool, MCP, package, repository, document, issue, and design output as
  untrusted input by default;
- detect instruction-shaped tool output and quarantine it before context
  assembly;
- build replayable injection and poisoning corpora with expected policy
  outcomes;
- configure publisher trust roots, key rotation, revocation, and transparency
  evidence;
- fuzz path, manifest, RPC, handoff, approval, and receipt parsers.

## Phase 12: operate Pandora as an agent platform

Phase 12 turns existing workers and CI pieces into one observable operating
system for local agents.

Next work:

- add agent CI/CD pipelines for package admission, tests, evaluations, canaries,
  and release promotion;
- let agents manage agents only through explicit budgets, leases, scopes, and
  review gates;
- add fleet health, queue depth, lease age, cost, and failure views without
  uploading prompts, outputs, secrets, or hidden reasoning;
- enforce budgets across repositories and dependent orchestration roles;
- promote beta, release-candidate, and stable channels from the same signed
  evidence set.

## Issue-sized contribution areas

Good public issues have one observable result, one authority statement, and
tests that prove both the success and failure paths. The current backlog is
best split into:

- route-conflict preview and package-authoring validation;
- desktop accessibility and native packaging checks;
- worker crash, daemon restart/reaping, lease expiry, and cancellation race tests;
- multi-repository partial-failure fixtures and reconciliation evidence;
- evaluation fixtures and scorecard views;
- memory provenance and revocation inspection;
- poisoning corpora, parser fuzz targets, and publisher trust design;
- release drill scripts and evidence indexing.

Changes to Parliament, Shadow Council, ReferenceMonitor, permit issuance,
constitutional Source bindings, or self-activation rules need an architecture
issue before code. Small tests, docs, adapters, inspection views, and
failure-handling patches are suitable first contributions.
