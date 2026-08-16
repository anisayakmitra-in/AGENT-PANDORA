# Efficiency Engine

Status: Shipped in the runtime contract.

`EfficiencyEngine` records bounded, task-class metrics for approved execution targets. `ObservabilityEngine` supplies usage and latency measurements; `EvaluationEngine` supplies verified completion outcomes; `EfficiencyEngine` keeps a rolling evidence window; `AdaptiveEngine` may use the resulting ranking when policy permits. Parliament and the reference monitor remain the authority for selection and effects.

The engine exposes three separate objectives:

- lowest measured cost;
- lowest measured latency;
- highest verified completion rate.

“Certainty” means historical completion evidence for the same bounded task class. It is not model confidence, a guarantee, or a reason to bypass evaluation or policy.

Samples store a task class and target identifier, not prompts, credentials, or hidden reasoning. The rolling window is bounded per target, and rankings use deterministic tie-breakers. The engine only ranks evidence: it cannot execute, authorize, install, or change permissions.
