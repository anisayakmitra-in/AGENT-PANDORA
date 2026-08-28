# Runtime inventory

Pandora's local service reports a read-only component inventory through
`runtime.engines`. The desktop **Runtime Inventory** surface groups those
records by category and exposes each component's inputs, outputs, invariants,
evidence classes, source modules, relationships, and documentation paths.

Inventory records describe compiled contracts. They are not health checks,
activation receipts, effect permits, or proof that a replaceable package is
trusted.

## Reported components

| Category | Components |
| --- | --- |
| Core authority | `ExecutionController`, `ReferenceMonitor` |
| Tools and context | `ToolEngine`, `ContextEngine`, `MemoryEngine`, `SkillEngine`, `MCP adapter` |
| Resilience | `ContextRecovery`, `FailoverProvider` |
| Self-improvement | `EvaluationEngine`, `EvolutionEngine`, `MutationEngine`, `AdaptiveEngine`, `CodingFeedbackLoop`, `EfficiencyEngine`, `SelfHealingEngine`, `ReplacementEngine`, `PopulationStrategy`, `GraphIntelligenceEngine` |
| Multi-agent execution | `OrchestrationEngine`, `FleetEngine` |
| Evidence | `ObservabilityEngine` |

The current inventory contains 22 components. `ContextRecovery` and
`FailoverProvider` are embedded resilience components, not independent
authorities:

- `ContextRecovery` is owned by `ContextEngine`. It follows a fixed sequence:
  prune low-value context, restore core context, rebuild from verified L1,
  retrieve fresh trusted evidence, reduce scope, or pause.
- `FailoverProvider` is controlled by `ExecutionController`. It is eligible only
  after a retryable primary failure. The fallback attempt receives a fresh
  policy decision, one-shot permit, receipt, and metrics record. Nested fallback
  is rejected.

## Authorities outside the component inventory

Parliament and Shadow Council remain separate constitutional authorities:

- Parliament decides policy.
- Shadow Council selects only approved Harness, Gene, and provider
  compositions.
- ReferenceMonitor alone issues exact one-shot effect permits.

Auto Route can read declared route hints from enabled Domain packages. That
metadata affects Harness selection only. Explicit selection wins, equal top
matches fail closed, and a package cannot use route metadata to select a Gene,
add capabilities, approve an effect, or issue a permit.

No inspected component can grant itself capabilities, change policy roots,
activate an evolution candidate, or bypass ReferenceMonitor.

## Adjacent inventories

The desktop inventory links to the other runtime-reported surfaces rather than
flattening their contracts into component records:

- Harness Lab: Harnesses, Genes, packages, plugins, authority, and receipts;
- Tools: registered effect contracts;
- Connections: providers, MCP servers, and package registries;
- Evolution: proposals, lineage, evaluations, activation, canary, and rollback;
- Audit: canonical runtime events and receipts.
