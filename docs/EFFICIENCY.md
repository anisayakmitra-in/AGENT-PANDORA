# Efficiency Engine

Status: Shipped in the runtime contract.

`EfficiencyEngine` records bounded, task-class metrics for approved execution targets. `ObservabilityEngine` supplies usage and latency measurements; `EvaluationEngine` supplies verified completion outcomes; `EfficiencyEngine` keeps a rolling evidence window; `AdaptiveEngine::select_with_efficiency` may use the resulting ranking when policy permits. Parliament and the reference monitor remain the authority for selection and effects.

The CLI persists the bounded evidence in its private data directory and can
inspect it with `pandora efficiency rank`. The persisted ledger contains only
execution IDs, bounded task and target labels, token counts, measured latency,
explicit cost evidence, completion state, and timestamps.

The engine exposes four separate objectives:

- lowest measured cost;
- lowest measured latency;
- lowest measured token usage;
- highest verified completion rate.

“Certainty” means historical completion evidence for the same bounded task class. It is not model confidence, a guarantee, or a reason to bypass evaluation or policy.

Samples store a task class and target identifier, not prompts, credentials, or hidden reasoning. The rolling window is bounded per target, and rankings use deterministic tie-breakers. The engine only ranks evidence: it cannot execute, authorize, install, or change permissions.

Cost evidence is explicit. A sample without provider pricing is not treated as a zero-cost run and is ranked after samples with known cost for the cost objective.

The default `AdaptiveEngine::select` path remains score-based for compatibility.
Evidence-ranked selection is opt-in and falls back to the existing score order for
approved candidates without matching history.
