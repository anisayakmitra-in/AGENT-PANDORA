# Efficiency Engine

Status: Shipped as a bounded CLI evidence ledger.

`EfficiencyEngine` ranks bounded task-class evidence recorded by CLI runs. Agent runs contribute one sample per provider attempt, including fallback attempts, with provider-reported token usage, elapsed latency, explicit operator-supplied cost evidence, and governed completion state. An attempt is counted as completed only when the overall task completed and that attempt succeeded. Direct runs contribute elapsed latency and completion state; they do not claim unavailable provider usage or cost. `EfficiencyEngine` keeps a rolling evidence window, and `AdaptiveEngine::select_with_efficiency` may use the resulting ranking when policy permits. `ObservabilityEngine` and `EvaluationEngine` remain separate derived views in this release. Parliament and the reference monitor remain the authority for selection and effects.

The CLI persists the bounded evidence in its private data directory and can
inspect it with `pandora efficiency rank`. The persisted ledger contains only
execution IDs, bounded task and target labels, token counts, measured latency,
explicit cost evidence, completion state, and timestamps. Fallback samples use
distinct attempt identities so multiple attempts from one execution remain
independently rankable.

The engine exposes four separate objectives:

- lowest measured cost;
- lowest measured latency;
- lowest measured token usage;
- highest verified completion rate.

“Certainty” means historical completion evidence for the same bounded task class. It is not model confidence, a guarantee, or a reason to bypass evaluation or policy.

Samples store a task class and target identifier, not prompts, credentials, or hidden reasoning. The rolling window is bounded per target, and rankings use deterministic tie-breakers. The engine only ranks evidence: it cannot execute, authorize, install, or change permissions.

Cost evidence is explicit. A run records known cost only when its provider
profile declares both input and output rates with
`--input-micros-per-million-tokens` and `--output-micros-per-million-tokens`.
Missing pricing remains unknown, never zero-cost, and is ranked after
known-cost samples for the cost objective. Fallback attempts use the pricing
profile of the provider that handled the attempt when one is configured.
Pricing is operator-supplied metadata, not inferred from a provider name or
response.

The default `AdaptiveEngine::select` path remains score-based for compatibility.
Evidence-ranked selection is opt-in and falls back to the existing score order for
approved candidates without matching history.

Agent and planning runs may opt into provider selection with
`--optimize cost|latency|tokens|certainty`. Selection matches evidence to an
explicitly configured `<provider>/<model>` profile and requires at least one
completed sample. Missing or unsuitable evidence preserves the active provider;
cost selection also requires explicit pricing evidence. Selection does not
change policy, permissions, credentials, or provider configuration.
