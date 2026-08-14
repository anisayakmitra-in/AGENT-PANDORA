# Governed evolution

Status: partial in the current `v2.0.x` preview line.

Pandora records improvement evidence without allowing the improvement system to authorize or activate itself.

## Shipped

- `ReflexionArtifact` stores a bounded, redacted summary, failure signals, and lesson tied to an execution.
- `MutationEngine` accepts GEPA proposals only in research mode. Production mode rejects mutation proposals.
- `EvolutionEngine` tracks proposal, evaluation, approval, and staging state.
- Approval requires policy, regression, and holdout evidence, Parliament approval, and candidate-artifact signature evidence.
- `ReplacementEngine` requires a passed canary and an idle execution boundary before activation.
- Activation produces a receipt. Rollback restores the proposal's base artifact and produces a receipt.

## Authority

Evolution records are evidence and workflow state. They do not grant permissions, mint effect permits, change policy, install packages, or execute code.

The package admission boundary remains responsible for validating artifact identity and signatures. This slice records the required evidence and binds it to the candidate artifact; it does not perform cryptographic verification.

Replacement is available only between executions registered with `ReplacementEngine`. A failed canary cannot activate, and an active replacement can be rolled back to its recorded base artifact.

## Not shipped

Autonomous code mutation, automatic promotion, hidden reasoning storage, and mid-execution replacement are not part of the current public contract.
