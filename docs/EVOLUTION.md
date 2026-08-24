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
- Approval requires policy, regression, and holdout evidence, Parliament approval, and candidate-artifact signature evidence.
- `ReplacementEngine` requires a passed canary and an idle execution boundary before activation.
- Activation produces a receipt. Rollback restores the proposal's base artifact and produces a receipt.

## Authority

Evolution records are evidence and workflow state. They do not grant permissions, mint effect permits, change policy, install packages, or execute code.

`PopulationStrategy` does not run mutation code, promote memory, authorize effects, or activate artifacts. It supplies bounded plans, precheck evidence, generation receipts, and lineage views to Pandora's existing evaluation and evolution paths.

The package admission boundary validates artifact identity and supported
Ed25519 signature evidence before recording a package. It does not establish
publisher trust, grant permissions, or make the artifact executable.

Replacement is available only between executions registered with `ReplacementEngine`. A failed canary cannot activate, and an active replacement can be rolled back to its recorded base artifact.

## Not shipped

Autonomous code mutation, automatic promotion, hidden reasoning storage, and mid-execution replacement are not part of the current public contract.
