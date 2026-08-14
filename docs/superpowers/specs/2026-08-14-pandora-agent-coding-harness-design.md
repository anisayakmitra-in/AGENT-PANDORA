# PANDORA-AGENT Coding Harness Design

## Status

Approved design scope for the first implementation slice.

PANDORA-AGENT is a new private repository. O-PANDORA remains a read-only
architecture reference. No O-PANDORA source files, generated binaries, or
legacy manifests are copied into the new repository.

## Goal

Ship a CLI-first coding agent that preserves Pandora's governed architecture:

```text
CLI
  -> PandoraRuntime
  -> ExecutionController
  -> ShadowCouncil
  -> Parliament
  -> ReferenceMonitor
  -> EffectExecutor
  -> operating-system effect
```

The first release supports one complete Coding Domain Harness. Its Genes can
inspect a workspace, propose changes, apply an approved patch, run bounded
verification, and report an auditable result.

## Architectural Ownership

### Parliament

Parliament is the policy decision authority. It evaluates an execution or
effect request and returns an allow, deny, or approval-required decision. It
does not discover Genes, execute tools, or mint a permit.

### Shadow Council

Shadow Council owns registration, enablement, capability discovery, routing,
and Harness/Gene ownership. It does not authorize operating-system effects.

### Harnesses

- Source Harness: augments one constitutional service. No Source Harness is
  enabled in the first coding slice.
- Meta Harness: coordinates other Harnesses. Reserved for later composition.
- Domain Harness: supplies roles, workflows, Genes, tools, and evaluators for
  one domain. The first implementation is `coding`.

### Genes

A Gene is a bounded capability selected by Shadow Council. A Gene may return a
pure result or an effect request, but it never receives raw filesystem,
process, network, or credential handles. Every effect request passes through
Reference Monitor and EffectExecutor.

### Reference Monitor

Reference Monitor is the sole permit issuer. A permit is bound to the
execution ID, session ID, principal, request digest, capability, target,
resource scope, policy version, expiry, and one-shot nonce. The executor
rejects missing, forged, expired, replayed, or mismatched permits.

### K-O Palace

K-O Palace remains a separate registry and artifact source. It does not own
Pandora activation, permissions, routing, runtime events, lockfiles, or
execution state. Registry integration is outside the first coding slice.

## Coding Domain Harness

The built-in `coding` Domain Harness declares these roles and Genes:

| Gene | Purpose | Effect class | Default policy |
|---|---|---|---|
| `workspace.read` | Read a bounded text file | Filesystem read | Allowed inside selected workspace |
| `workspace.search` | Search tracked workspace files | Filesystem read | Allowed inside selected workspace |
| `patch.apply` | Apply an exact, reviewed patch | Filesystem write | Requires approval |
| `verification.run` | Run configured project checks | Process execution | Requires approval |
| `change.review` | Summarize the proposed diff and checks | Filesystem read | Allowed inside selected workspace |

The Harness may use different approved model connections for planning, making,
review, and verification in later releases. The first release uses one active
provider connection and keeps role metadata explicit for forward compatibility.

The initial coding flow is:

1. Accept a task and selected workspace.
2. Parliament validates the task, workspace, budgets, and policy.
3. Shadow Council selects the Coding Harness and allowed Genes.
4. The provider proposes a structured plan.
5. Read/search Genes collect bounded workspace evidence.
6. The provider proposes a patch as a structured artifact.
7. Parliament evaluates the patch effect request.
8. Reference Monitor mints a one-shot write permit only after approval.
9. `patch.apply` applies the exact permit-bound patch.
10. `verification.run` executes only an approved command profile.
11. `change.review` produces a redacted result and immutable receipt.

No model output, Skill, Harness, or Gene can approve its own effect.

## CLI v0.1

The initial public commands are deliberately small:

```text
pandora setup
pandora doctor
pandora run <task> [--workspace <path>] [--model <name>]
pandora sessions list
pandora sessions show <id>
pandora approve <id>
pandora reject <id>
pandora trace <id>
pandora harness list
pandora harness inspect coding
pandora gene list
pandora --json <command>
```

Every command has stable exit codes and a machine-readable JSON form. Human
output is for operators; automation must consume JSON.

`pandora setup` configures one provider connection. Credentials are read from
stdin or the operating-system credential store and are never stored in task,
session, package, or trace records.

## Canonical v0.1 Data

The first contracts are limited to:

- `ExecutionRequest`
- `ExecutionId` and `SessionId`
- `HarnessManifest` and `GeneManifest`
- `EffectRequest`
- `EffectPermit`
- `EffectReceipt`
- `ApprovalRequest` and terminal approval status
- `RuntimeEvent` envelope
- `SessionSummary`
- `ProviderConnection` without secret material

There is one definition for each contract. Compatibility adapters are not
created until a real external boundary requires one.

## Persistence and Memory

SQLite is the local persistence baseline. Writes for sessions, approvals,
effect receipts, and runtime events are transactional and correlation IDs are
mandatory.

Memory uses explicit layers:

- `EphemeralTrace`: bounded in-memory frames with expiry.
- `DistilledExecution`: redacted outcomes, decisions, failures, benchmarks,
  and provenance.
- `EvolutionaryMemory`: approved lessons, lineage, policy decisions, and
  replacement records only.

Hidden chain-of-thought, raw credentials, and unredacted provider secrets are
never persisted.

## Future Architecture Markers

These systems remain part of Pandora's architecture but are not empty v0.1
crates:

- ContextEngine: context budgeting, provenance, retrieval, cache policy, and
  context-rot recovery.
- MemoryEngine: the three explicit memory layers.
- ToolEngine: schemas, argument validation, idempotency, and effect requests.
- EvaluationEngine: trajectory, outcome, policy, regression, adversarial, and
  human evaluation.
- ObservabilityEngine: traces, spans, tokens, latency, cost, errors, and drift.
- OrchestrationEngine: role scheduling and bounded workflow composition.
- AdaptiveEngine: selection among already approved options.
- GraphIntelligenceEngine: code, knowledge, review, and architecture graphs.
- FleetEngine: local and later authenticated remote workers.
- MutationEngine and EvolutionEngine: evidence-backed proposals only.
- ReplacementEngine: approved, signed, canaried, reversible DSR only.
- SelfHealingEngine: allowlisted operational recovery only.
- Reflexion: bounded post-session guidance.
- Voyager-style learning: candidate Skills, never direct code activation.
- LATS/MCTS: research-profile planner with hard budgets.

GEPA, RSI, and DSR cannot modify Parliament, Reference Monitor, executor
policy, approval records, signing authority, or rollback mechanisms.

## Provider and Inference Boundary

PANDORA-AGENT owns provider selection policy and records provider telemetry. It
does not claim control over a hosted provider's internal KV cache, batching,
attention implementation, quantization, or decode scheduler. Those are
capability declarations and measured profiles at the provider boundary.

The provider contract must expose structured responses, tool-call arguments,
usage, latency, and failure classification. Malformed structured output is
handled by bounded repair and fallback; repeated repair failure terminates the
turn safely.

## Security Invariants

- No valid permit means no operating-system effect.
- Permits are one-shot, expiry-aware, request-bound, and replay-resistant.
- Workspace paths are canonicalized and contained before access.
- Shell execution is not free-form in v0.1; verification uses allowlisted
  command profiles.
- Third-party native packages are not executed automatically.
- Skills cannot execute scripts directly; scripts must become ToolEngine
  requests.
- Tool output is untrusted input to the provider.
- Limits apply to turns, tools, tokens, duration, output bytes, and cost.
- Every attempted effect produces a receipt, including denial.

## Testing and Release Gates

Every implementation commit must pass:

```text
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Required v0.1 scenarios include provider setup, clean workspace selection,
read/search success, patch approval, rejected patch, expired approval,
replayed permit, path escape rejection, command-profile rejection, malformed
provider output, session resume metadata, JSON output, and receipt replay.

The first release is `v0.1.0`. It is a source-build/private development
release until signed Windows, macOS, and Linux artifacts pass clean-machine
installation tests. No desktop, Android, remote Fleet, or K-O Palace product
support is claimed by v0.1.0.

## Explicit Non-Goals

The first slice does not include:

- a desktop client;
- Android or Termux support;
- remote execution;
- marketplace installation;
- arbitrary native third-party Genes;
- unrestricted shell access;
- automatic self-modification;
- production DSR activation;
- all historical domain Harnesses;
- a second runtime or event model.
