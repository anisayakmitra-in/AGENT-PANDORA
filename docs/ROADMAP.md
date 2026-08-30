# Pandora roadmap

Pandora is roughly 60-70% complete against the local agent platform described
in this roadmap. The center of that range is about 65%. This is a planning
estimate from the source tree, tests, desktop, workflows, and documented release
gates on 2026-08-28. It is not a release claim.

The execution and authority core is ahead of the product loops around it.
Parliament, Shadow Council, ReferenceMonitor, exact Harness and Gene bindings,
one-shot permits, receipts, package admission, and governed replacement already
exist. The largest unfinished areas are automatic evaluation loops,
cross-session memory policy, hostile-input testing, fleet
operations, and signed native releases.

## Status by phase

| Phase | Estimate | Shipped in the source tree | Work still open |
| --- | ---: | --- | --- |
| 6. Production readiness | 85-90% | scoped identity, automatic local device trust, encrypted secrets, local telemetry and crash records, encrypted backup and restore, fresh-runner install/update/rollback/backup/restore/uninstall drills, update channels, release workflows, checksum signature verification, release evidence index, CodeQL, dependency audits | signed stable artifacts, real clean-machine release proof on every advertised platform, installer rollback exercises with the stable artifact |
| 7. Runtime scale and orchestration | 90-95% | persistent prompt-context cache, headless jobs, bounded parallel subagents, exact-commit worktrees, durable orchestration claims and receipts, interruption and resume rules, multi-repository plans, fleet leases, budgets, execution-bound lease renewal, and durable supervisor state with PID-bound worker heartbeats, process-wide execution leases for headless jobs and subagents, lease gating, stale-supervisor reconciliation without replay, bounded stale reaping, atomic PID-bound restart handoff, atomic cross-process quiescence guards, bounded independently launched job watch windows, long-lived local daemon workers with explicit drain/stop protocol, cross-process crash reconciliation/restart evidence, bounded staggered-producer soak coverage, cancellation/provider-return restart evidence, and combined cross-process worker-operations recovery acceptance | retained long-duration and multi-platform worker soak evidence |
| 8. Agent experience and disclosure | 80-88% | native desktop source, Command and Council inspection, background runs, runtime inventory, Harness Lab, package lifecycle, package manifest workbench, BYOK providers and models, MCP configuration, pinned GitHub packages, active custom Domain and Meta Harnesses, WebAssembly Genes, custom Auto Route contracts, optional built-in Domain and Meta replacement, encrypted-vault package key generation and atomic local manifest signing | broader Skill and provider package lifecycles, desktop accessibility pass, native installer release proof |
| 9. Evaluation-driven loops | 85-90% | trajectory, outcome, policy, regression, adversarial, golden, and holdout evaluation; coding feedback; research-only mutation and population strategies; durable evolution state; canary activation and rollback; versioned evidence-derived zero-failure canary policy; read-only durable per-session evaluation scorecards with a fail-on-non-passed CI gate; durable schedules with a bounded local registry of validated suite definitions; typed prompt/Skill/workflow/WebAssembly Gene target metadata; durable failure-derived regression candidates; explicit review-gated suite admission; task-backed suites executed through governed Controller adapters with bounded evidence | self-healing test generation beyond metadata candidates; automated canary scheduling and rollout policy; quality gates for every artifact class |
| 10. Memory consolidation | 70-75% | scoped L0, L1, and L2 records; durable recall; approval-gated promotion; revocation, audit, and compaction; deterministic evidence-bound synthesis; bounded CLI synthesis preview and commit with stale-snapshot checks; bounded read-only CLI provenance graphs; explicit same-tenant/workspace/provider cross-session L1 consolidation with dry-run and hashed source provenance | cross-project consolidation policy, scheduled synthesis, desktop removal views, source graph visualization and retention controls |
| 11. Adversarial resilience | 65-72% | path confinement, symlink checks, secret redaction, replay protection, exact signatures and hashes, fail-closed package and permit checks, adversarial evaluation primitives, typed context-origin metadata plus source-labelled deterministic quarantine for high-confidence instruction-shaped tool and adapter output, replayable hostile-output corpus coverage, durable publisher trust roots with active-key admission, rotation, revocation, and fail-closed reload behavior | prompt-injection corpus across every input source, transparency evidence for trust changes, fuzzing and hostile multi-agent handoff suites |
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

The combined Phase 7 worker-operations acceptance profile now runs three
independent producer streams and 18 governed jobs under live queue pressure,
force-stops the first daemon, reconciles its stale PID and expired lease from a
fresh CLI, and binds a second daemon at a new PID and generation. Durable
snapshots prove exactly one terminal result, session, evaluation, rollout, and
effect receipt per job before and after another fresh worker observes the empty
queue. The same fixture drives a partial two-repository role failure through
independently restarted CLI processes, preserves the completed planner receipt
and active maker role, rejects duplicate or mismatched completion, and keeps
resume blocked for explicit receipt reconciliation.

This evidence does not widen authority:
`ExecutionController -> Parliament -> ReferenceMonitor -> executor -> receipt`
remains the only effect path. Workers, Fleet, and Orchestration cannot add
capabilities, issue permits, or replay uncertain effects.

Next work:

- run and retain the documented opt-in 10-minute worker-operations soak on
  every advertised platform;
- retain multi-hour or day-scale operator evidence before claiming workers stay
  healthy for days.

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

- complete keyboard, screen-reader, reduced-motion, scaling, and high-contrast
  checks in the native desktop;
- define separate admission rules before Provider, Skill, Source, or generic
  packages can become active.

## Phase 9: connect evidence into repeatable loops

The evaluator and governed activation lifecycle exist, but operators still
drive most transitions. Phase 9 makes those loops repeatable without letting
evaluation approve its own candidate.

The CLI now exposes a read-only scorecard over persisted session receipts. It
aggregates result status and scores by evaluation kind and emits a deterministic
digest, without rerunning evaluation or granting approval. CI can opt into
`--fail-on-non-passed` to fail closed on failed or human-review results while
returning the same scorecard evidence.

Next work:

- typed target bindings, bounded task labels, and durable failure-derived regression candidates now cover prompt, Skill, workflow, and WebAssembly Gene cases; explicit review is required before a candidate can register a suite, while the runner still evaluates redacted evidence and does not execute target tasks;
- generate regression tests from verified failures and require review before
  they enter a suite;
- record cost, latency, stability, and quality scorecards per candidate;
- automate policy-driven canary scheduling against fixed budgets and pause for Parliament approval before activation;
- expose every rejection, retry, and rollback as durable evidence.

## Phase 10: make memory useful across work

Pandora already stores scoped summaries and can synthesize an evidence-bound L1
record. It does not yet decide when lessons may cross a session or project.

Next work:

- extend the explicit same-workspace cross-session boundary with cross-project policy and conflict rules;
- add scheduled synthesis with stale-evidence checks and durable synthesis results;
- expose source records, promotion approval, revocation, and compaction in the
  desktop;
- add retention controls and secure-erasure guidance for databases, backups,
  and storage snapshots;
- connect evolution lineage queries to the exact memory evidence IDs that
  shaped a candidate.

## Phase 11: treat content as hostile

Pandora already confines effects and validates identity. Phase 11 adds defenses
against validly delivered but malicious content.

Tool results remain untrusted by default. A small high-confidence marker set now
quarantines instruction-shaped output before it reaches provider context, while
retaining only its digest and byte count for evidence. Ordinary tool output keeps
its bounded content, and persisted quarantined records remain quarantined on
resume. This marker set is a guardrail, not a complete injection detector.
The replayable runtime corpus covers each high-confidence marker plus a benign
control case, so changes to the boundary have an explicit regression oracle.
User-selected retrieved attachments use the same marker boundary before system
context assembly and durable attachment persistence, preserving the original
digest and byte count without forwarding hostile text. Context manifests now also
carry typed, snake-case origin kinds for runtime, memory, Skill, user selection,
tool, MCP, package, repository, document, issue, design, handoff, and external
producers; the kind is inspection evidence and does not grant trust.

Next work:

- preserve the common untrusted boundary while wiring explicit origin labels
  through each tool, MCP, package, repository, document, issue, design, and
  handoff adapter;
- expand the replayable injection and poisoning corpora across each source with
  expected policy outcomes and benign controls;
- add transparency evidence for trust-root changes and package admission decisions;
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
- poisoning corpora, parser fuzz targets, and publisher trust transparency evidence;
- release drill scripts and evidence indexing.

Changes to Parliament, Shadow Council, ReferenceMonitor, permit issuance,
constitutional Source bindings, or self-activation rules need an architecture
issue before code. Small tests, docs, adapters, inspection views, and
failure-handling patches are suitable first contributions.
