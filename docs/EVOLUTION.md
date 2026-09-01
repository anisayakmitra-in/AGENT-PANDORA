# Governed evolution

Status: partial in the current `v2.0.x` preview line.

Pandora records improvement evidence without allowing the improvement system to authorize or activate itself.

## Shipped

- `ReflexionArtifact` stores a bounded, redacted summary, failure signals, and lesson tied to an execution.
- `MutationEngine` accepts GEPA proposals only in research mode. Production mode rejects mutation proposals.
- `PopulationStrategy` is research-only. It ranks viable candidates deterministically by score and novelty, then builds bounded mutation batches from training failures. Mutation requests carry the holdout digest and count, but never holdout failure content.
- Policy and regression prechecks run before full candidate evaluation. A generation commits only after every outcome validates, and its receipt accounts for accepted, rejected, and precheck-rejected candidates plus measured usage.
- Redacted lineage attempts are tied to committed generation receipts. They enter L1 memory under the exact tenant, workspace, session, and provider scope. L2 promotion still requires the existing `MemoryEngine` approval path. Ancestor and neighborhood queries have depth, record, and byte limits.
- `EvolutionEngine` tracks proposal, evaluation, approval, and staging state.
- The evolution record store persists those bounded records in SQLite and restores them after restart. `pandora evolution list` and `pandora evolution inspect` expose read-only operator views without exposing signature material.
- `pandora evolution evaluate` runs a bounded holdout set against an existing proposal and records trajectory, outcome, policy, and regression evidence. The report digest excludes raw outputs, expected outputs, and baselines.
- `pandora evolution submit` records a bounded proposal in the durable store. Submission is evidence intake only; it cannot approve, activate, or execute a candidate.
- `pandora evolution generate` is the research-only intake path for prompt, Skill, workflow, and WASM Gene candidates. It sends a bounded base artifact, structured evaluation/feedback summaries, and explicitly approved internal memories to the configured provider. Every included memory carries its exact stable ID; the canonical ID list is hashed into evidence and persisted on the proposal. The provider response must be exact JSON, cannot contain tool calls, and becomes only a proposed artifact with exact-byte provenance in `research-artifacts.sqlite3`.
- `pandora evolution approve --input <path>` records Parliament approval and candidate signature evidence after evaluation gates pass. Approval does not activate a candidate.
- Approval requires policy, regression, and holdout evidence, Parliament approval, and candidate-artifact signature evidence.
- `pandora evolution stage` and `pandora evolution canary` establish the production candidate boundary. `pandora evolution rollout` then persists canary, limited, expanded, and complete rollout stages before the existing activation command can run.
- Activation requires both the base and candidate content hashes to exist in the admitted package store for package and WASM Gene candidates. Prompt, Skill, and workflow research candidates validate their durable exact-byte provenance instead; their catalog activation remains non-executable until a future runtime consumer explicitly reads that artifact class. Admission still grants no runtime authority.
- `ArtifactCatalog` persists active base-to-candidate bindings, resolves bounded replacement chains, rejects cycles and duplicate bases, and requires dependent replacements to roll back first.
- `ReplacementEngine` requires a passed canary and an idle execution boundary registered with that engine before activation. Activation and rollback produce typed receipts, and failed catalog changes compensate by rolling evolution state back closed.
- Admitted custom WebAssembly Genes resolve the active catalog chain once while Pandora assembles the selected Domain Harness. The resulting execution profile keeps the base Gene ID and version while binding the exact resolved artifact hash. A catalog change can affect a later profile, but cannot replace the module inside an in-flight profile.
- The CLI, local service, and desktop expose the proposal's exact memory-evidence IDs alongside its evidence digest. Legacy proposals deserialize with an empty list. The service and desktop expose active catalog bindings read-only; they do not expose approval or staging controls, and desktop activation or rollback remains an explicit exact-confirmation operation.

## Authority

Evolution records are evidence and workflow state. They do not grant permissions, mint effect permits, change policy, install packages, or execute code.

`PopulationStrategy` does not run mutation code, promote memory, authorize effects, or activate artifacts. It supplies bounded plans, precheck evidence, generation receipts, and lineage views to Pandora's existing evaluation and evolution paths.

The package admission boundary validates artifact identity and supported
Ed25519 signature evidence before recording a package. It does not establish
publisher trust, grant permissions, or make the artifact executable.

The `ReplacementEngine` serializes its local lifecycle transition behind an idle boundary. A failed canary or an artifact absent from the admitted package store cannot activate, and an active replacement can be rolled back to its recorded base artifact. Chained replacements unwind from the tip so rollback cannot strand an active dependent. Runtime consumption uses immutable execution-profile snapshots: concurrent custom Wasm runs observe either the catalog state before or after a committed transition, never a mid-run substitution.

## Governed staged rollout

A rollout is an optional strict gate added after the existing canary result. Once configured, it is durable and cannot be replaced. Its binding contains the exact source commit, candidate artifact digest, release channel, and proposal evidence digest. The evidence digest must match the proposal, and the channel uses the same closed `beta`, `release-candidate`, or `stable` vocabulary as release promotion.

Every rollout contains exactly four ordered stages: `canary`, `limited`, `expanded`, and `complete`. Each stage has explicit maximum cost, elapsed duration, failure count, and p95 latency plus minimum quality and stability scores. A recorded scorecard either moves the stage to `awaiting_approval` or fails it closed. Failed or rejected stages have at most three explicit retries.

Promotion requires a non-expired human approval bound to the proposal, current and next stage, exact rollout binding, and latest scorecard digest. The scorecard actor must be its named evaluator, and that evaluator principal cannot be the approver. Approval identities and transition identities are SHA-256 values. Every transition also stores an internally computed request fingerprint: an exact payload retry is idempotent, while using the same identity for another action or changed payload is rejected. Pause, resume, reject, retry, promotion, and rollback each append bounded transition evidence to the same SQLite proposal record. Rejection and rollback consume any pending approval so it can never be reused.

The final `complete` stage also needs a passing scorecard and separate human approval. Its promotion sets `activation_ready`; it does not activate the artifact. `pandora evolution activate` remains the only artifact-catalog activation path, with its existing quiescence, admission, backup, and replacement receipts. A rollout rollback changes the rollout state only and cannot replay an uncertain artifact effect.

## Not shipped

Autonomous code mutation, automatic promotion, hidden reasoning storage, and mid-execution replacement are not part of the current public contract.

Built-in compiled Harnesses and Genes do not resolve artifact identities through `ArtifactCatalog`; only admitted custom Wasm Gene dependencies consume active bindings today. The direct ReplacementEngine API still tracks in-process executions, while the service and CLI evolution mutation paths acquire a durable Fleet quiescence guard that blocks new leases across processes until the mutation scope exits. Safety does not depend on mutating running modules: every custom Wasm run snapshots and authorizes one exact resolved artifact, and activation affects only profiles assembled after the catalog transaction commits.
