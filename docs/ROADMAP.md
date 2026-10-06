# Pandora roadmap

Pandora is a CLI project. The desktop product was deleted on 2026-10-05 and
left no tree, build, packaging, or release path behind.

Completion is estimated from the CLI source tree, tests, workflows, and
documented release gates. It is not a release claim.

The execution and authority core is ahead of the product loops around it.
Parliament, Shadow Council, ReferenceMonitor, exact Harness and Gene bindings,
one-shot permits, receipts, package admission, and governed replacement already
exist. The largest unfinished CLI release gates are published-channel evidence
(Homebrew, Scoop, and winget, which need external repositories this project does
not own) and published stable rollback evidence. Platform code signing is not on
the roadmap for any channel: release binaries are not OS-signed, and integrity is
carried by checksums, a keyless cosign signature, and build attestations.

## Status by phase

| Phase | Estimate | Shipped in the source tree | Work still open |
| --- | ---: | --- | --- |
| 6. Production readiness | 95-97% | scoped identity, automatic local device trust, encrypted secrets, local telemetry and crash records, encrypted backup and restore, retained fresh-runner install/update/rollback/backup/restore/uninstall evidence, synthetic two-version native installer upgrade and rollback drills, update channels, protected release publication with human approval, checksum signature verification enforced at every published-verification call site, keyless cosign signature over the checksum manifest, GitHub build attestations verified on published binaries, cosign- and checksum-rendered Homebrew, Scoop, and winget manifests, cargo-binstall metadata, asserted macOS ad-hoc signature with no Developer ID claim, an explicit `platform_signing: not_applicable` posture in release evidence, release and stable-rollback evidence indexes, CodeQL, dependency audits | accepted real native accessibility records, published RC/stable lifecycle evidence, stable-to-stable rollback proof, publication to a Homebrew tap, a Scoop bucket, and winget-pkgs, which each need an external repository this project does not own |
| 7. Runtime scale and orchestration | 100% | persistent prompt-context cache, headless jobs, bounded parallel subagents, exact-commit worktrees, durable orchestration claims and receipts, interruption and evidence-bound reconciliation rules, multi-repository plans, transactionally enforced aggregate token/tool/time/cost budgets, atomic role reservations and receipt-linked usage settlement, conservative unknown-cost enforcement, fleet leases, execution-bound lease renewal, durable supervisor state with PID-bound worker heartbeats, process-wide execution leases for headless jobs and subagents, lease gating, stale-supervisor reconciliation without replay, bounded stale reaping, atomic PID-bound restart handoff, atomic cross-process quiescence guards, bounded independently launched job watch windows, long-lived local daemon workers with explicit drain/stop protocol, cross-process crash reconciliation/restart evidence, bounded staggered-producer soak coverage, cancellation/provider-return restart evidence, combined cross-process worker-operations recovery acceptance, and successful retained ten-minute and two-hour four-platform campaigns plus checkpointed eight-hour and twenty-four-hour profiles | none in the defined release gate; longer soak campaigns remain continuous assurance work |
| 8. Modular product surface | 100% | bounded Domain Harness route hints with longest-match auto route and fail-closed ties, package replacement of built-in Domain IDs only by exact verified signature, runtime inventory, Harness Lab local Skill lifecycle, package and Skill lifecycle, provider and MCP configuration, read-only registry and pinned-GitHub discovery into an inert cache, offline verification with no network, explicit admission, revocation quarantine, and CLI/TUI trust inspection | none in the defined source-tree scope; registry growth and longer assurance campaigns remain continuous work |
| 9. Evaluation-driven loops | 94-96% | trajectory, outcome, policy, regression, adversarial, golden, and holdout evaluation; coding feedback; research-only mutation and population strategies; durable evolution state; canary activation and rollback; versioned evidence-derived zero-failure canary policy; read-only durable per-session evaluation scorecards with a fail-on-non-passed CI gate; durable schedules with a bounded local registry of validated suite definitions; typed prompt/Skill/workflow/WebAssembly Gene target metadata; durable failure-derived regression candidates; explicit review-gated suite admission; governed scheduled execution of evidence-backed and task-backed suites; proposal-bound one-shot canary scheduling; exact-bound canary, limited, expanded, and complete rollout stages with cost, duration, failure, quality, latency, and stability limits; human approval; pause/resume/reject/retry/rollback evidence; CLI and TUI controls; deterministic artifact-class scorecards for typed evaluation reports; accepted-candidate regression fixture generation with exact source revalidation; and protected tag creation on the existing activation and publication paths | none in the defined source-tree scope; future artifact classes and longer assurance campaigns remain continuous work |
| 10. Memory consolidation | 100% | scoped L0, L1, and L2 records; durable recall; approval-gated promotion; revocation and audit; deterministic evidence-bound synthesis with stale-snapshot checks; bounded provenance graphs; versioned same-tenant/provider cross-session and explicit cross-project L1 consolidation with reject/keep-target conflict rules; durable leased synthesis schedules and run history; exact digest-bound memory IDs on evolution candidates; source, promotion, provenance, audit, revocation, schedule, transfer-policy disclosure, and typed retention-compaction controls; logical compaction that retains tombstones and audit evidence with explicit secure-erasure guidance; versioned local/AWS/Azure/GCP backup-expiry, snapshot-removal, and key-destruction manifests; non-mutating preview; append-only idempotent operator-attested receipts; CLI and TUI workflow and evidence views | none in the defined Phase 10 source-tree scope; provider control-plane actions and independent verification remain external by design |
| 11. Adversarial resilience | 100% | path confinement, symlink checks, secret redaction, replay protection, exact signatures and hashes, fail-closed package and permit checks, adversarial evaluation primitives, typed context-origin metadata, one shared revalidating quarantine boundary across all adapter origins, replayable hostile and benign corpus coverage, hostile multi-hop handoff persistence tests, append-only hash-chained publisher and admission transparency evidence, and six bounded production-parser fuzz targets with CI smoke runs | none in the defined Phase 11 source-tree scope; corpus growth and longer fuzz campaigns remain continuous assurance work |
| 12. Agent operations | 92-95% | three-platform CI, release and security workflows, bounded agent workers, orchestration receipts, supervisor controls, local fleet records, transactionally enforced multi-repository budgets with measured receipt usage and explicit unknown-cost accounting, one privacy-safe CLI/TUI Fleet dashboard, a staged SDK package/evaluation/canary pipeline, protected beta/release-candidate/stable promotion, and protected tag-driven release publication | signed RC and stable publication with real platform credentials and human approval |

The ranges separate code presence from operating proof. A component can be
implemented and tested locally while its clean-machine release, failure
recovery, or sustained-load evidence is still missing.

## Phase 6: close the release boundary

The local security controls exist. Phase 6 closes when a tagged native release
passes the documented gates on Windows, macOS, and Linux with no manual
security exception; the current Linux GTK dependency is source-patched and
provenance-bound until the supported upstream stack advances.

Next work:

- collect the exact-commit native NVDA, VoiceOver, and Orca records;
- publish the rendered Homebrew, Scoop, and winget manifests, which each need an
  external repository this project does not own;
- test recovery after interruption during update, restore, and catalog
  activation;
- review the generated release evidence index before publishing, then close the
  stable-to-stable rollback record with the first legitimate patch release.

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
fresh CLI, and binds a second daemon at a new PID and generation. Fresh CLI
inspections cover every terminal job and session; captured supervisor snapshots
make both generations inspectable, and the final Fleet inspection proves every
process lease is released. The same fixture drives a partial two-repository
role failure through independently restarted CLI processes, preserves the
completed planner receipt and active maker reservation, rejects duplicate or
mismatched completion, and proves that resume fails closed until an operator
records receipt-bound partial usage and reconciliation evidence. The safe
transition releases unused capacity exactly once, after which a replacement
worker can retry only inside the remaining aggregate ceiling. Normal CI remains
one bounded round; the
documented opt-in PANDORA_PHASE7_SOAK_ROUNDS multiplier adds bounded recovery
rounds without changing any authority boundary.

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

A manifest inspector previews the closed package vocabulary, exact JSON shape,
bounded Domain route hints, Meta composition, dependency declarations, and
unverified trust posture. Copying the JSON is the only mutation it performs; it
does not sign, admit, enable, publish, store private keys, or grant authority.
The existing admission and lifecycle boundaries remain the only path to durable
package state. It also previews exact route-hint overlaps across the local
catalog before a Domain is enabled. This is advisory evidence only; runtime
routing still uses active admitted bindings, explicit user selection wins, and
ambiguous ties fail closed.

The Harness Lab exposes SkillEngine's local lifecycle: install from an
absolute directory, inspect, enable, disable, suspend, remove with an exact
confirmation, and restore into the disabled state. Connections can select an
existing Provider profile. Both paths require a local-service restart before a
running controller sees the new configuration. Neither path grants runtime
authority.

Signed remote distribution now covers Gene, Domain Harness, Meta Harness, Skill,
and Provider kinds. Discovery is read-only; registry or full-commit GitHub download
verifies one exact Official manifest and artifact into an inert durable cache.
Publisher/key identity, manifest and artifact digests, source revision, trust
state, and admission binding are visible in CLI and TUI. Offline
verification uses no network. Exact replay is idempotent, while substitution,
downgrade, traversal, missing dependencies, untrusted publishers, and revoked
keys fail closed. Confirmed admission still leaves packages and Skills disabled
and Providers inactive; their existing enable or selection paths remain separate.
Revocation removes distribution bindings and suspends or quarantines matching
managed Skills and Providers.

Next work:

- retain signed-release and real-user update, rollback, and uninstall evidence
  on every advertised platform; tagged releases now re-download checksum-bound
  the published CLI artifact on fresh runners and prove bounded extraction,
  launch, and sandbox cleanup;
- collect and admit the four exact-commit native NVDA, VoiceOver, and Orca
  graphical-session records; automated 100%, 150%, and 200% evidence and
  clean-runner lifecycle JSON are retained already;
- exercise a real stable release end to end: human approval, cosign-signed
  checksum manifest, build attestations, and the published rollback drill. No
  platform signing credential is involved.

## Phase 9: connect evidence into repeatable loops

The evaluator and governed activation lifecycle exist, but operators still
drive most transitions. Phase 9 makes those loops repeatable without letting
evaluation approve its own candidate.

The CLI now exposes a read-only scorecard over persisted session receipts. It
aggregates result status and scores by evaluation kind and emits a deterministic
digest, without rerunning evaluation or granting approval. CI can opt into
`--fail-on-non-passed` to fail closed on failed or human-review results while
returning the same scorecard evidence.

Registered suites now share one durable scheduled execution path. Evidence
cases remain deterministic; task-backed cases use the governed Controller
adapter. A staged proposal can bind a one-shot occurrence that retains the
suite report digest and case counts, derives the production zero-failure
canary result, and stops at `canary_passed`. Evaluation still cannot approve or
activate its own candidate.

The governed rollout now starts after the existing one-shot canary. It records
four explicit stages with cost, duration, failure, quality, latency, and
stability limits. Every stage needs passing evidence and a separate exact-bound
human approval; automated evaluation cannot approve its own scorecard. Pause,
resume, rejection, bounded retry, promotion, and rollback are durable,
idempotent transitions. Completion opens the existing activation gate rather
than adding another activation path.

Phase status: complete for the defined source-tree scope. Regression fixture
generation remains inert until an operator registers and runs the accepted
candidate through the governed evaluation adapter; scorecards remain evidence
only and cannot approve or activate candidates.

## Phase 10: make memory useful across work

Pandora stores scoped summaries, synthesizes evidence-bound L1 records, and
uses a versioned transfer boundary before one L1 lesson crosses a session or
project. Cross-project transfer requires exact workspace IDs and an explicit
reject or keep-target conflict rule; it remains denied across tenants or
providers and cannot overwrite or reuse a tombstoned identity.

Durable synthesis schedules now re-check source evidence before commit and keep
bounded worker-owned run history. The CLI exposes records, source graphs,
audit, revocation, and schedules. Its retention panel previews an exact timestamp
and requires typed confirmation before compacting only already-revoked logical
records; tombstones and audit evidence remain, and storage-level erasure is
explicitly outside that operation.

Research evolution evidence now carries the exact canonical IDs of all included
memory records. Those IDs are covered by the evidence digest, persisted on the
candidate proposal, and visible in CLI, service, and lineage inspection.

Storage lifecycle policy version 1 closes the remaining evidence gap. It uses a
closed provider/action field matrix for local filesystem, AWS S3, Azure Blob,
and Google Cloud Storage backup expiry, snapshot removal, and encryption-key
destruction. Preview does not open the ledger. Explicit record writes an
append-only, digest-bound operator attestation; exact retry is idempotent and a
conflicting evidence ID fails closed. CLI list/inspect, TUI guidance, and the
evidence views all retain the same boundary: the runtime did not
perform the provider action and does not guarantee secure erasure.

Phase status: complete for the defined source-tree scope. Provider control-plane
actions, provider response verification, legal holds, replication, soft-delete,
and cryptographic-erasure semantics remain external operator responsibilities,
not hidden unfinished runtime authority.

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

The common guard now maps tool, MCP, package, repository, document, issue,
design, agent-handoff, browser, and generic external adapters onto the typed
origin vocabulary. It revalidates persisted and forwarded envelopes, so an
attacker cannot label hostile content as already normalized. The replayable
corpus applies hostile and benign controls to every origin and includes a
multi-hop persisted handoff regression.

Trust-root additions and revocations and every package admission decision now
append safe evidence to a bounded SQLite ledger. A SHA-256 predecessor chain
and database triggers make mutation or deletion detectable and fail closed.
CLI and TUI guidance expose the same read-only evidence without
granting package or execution authority.

Six `cargo-fuzz` targets drive the production path, package-manifest, MCP RPC,
orchestration handoff, approval, and persisted effect-receipt parsers. Seed
corpora and a bounded Ubuntu CI job keep the targets replayable. This does not
claim that marker matching or bounded fuzzing detects every malicious input.

Phase status: complete for the defined source-tree scope. New adapter kinds
must use the same origin and guard contract, and longer fuzz campaigns and
corpus expansion remain ongoing assurance work rather than hidden authority.

## Phase 12: operate Pandora as an agent platform

Phase 12 turns existing workers and CI pieces into one observable operating
system for local agents.

Next work:

- configure required reviewers on all three protected promotion environments
  and execute retained beta, release-candidate, and stable promotion drills;
- run the shared promotion and publication evidence chain on a real stable tag,
  with the cosign-signed manifest and build attestations verified end to end.

The aggregate execution-budget gate is source-complete. Every submitted role
has a durable reservation, dispatch and settlement are atomic, token/tool/time
usage is receipt-backed, provider cost remains explicitly unknown when absent,
and unknown cost consumes the full reservation for enforcement. The same
privacy-safe read model is visible through CLI and TUI surfaces.

## Issue-sized contribution areas

Good public issues have one observable result, one authority statement, and
tests that prove both the success and failure paths. The current backlog is
best split into:

- route-conflict preview and package-authoring validation;
- native CLI lifecycle and accessibility-of-terminal checks;
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
