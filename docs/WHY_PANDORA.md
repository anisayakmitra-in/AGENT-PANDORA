# Why Pandora?

Pandora is an open-source agent runtime for work that needs an accountable
execution boundary, not only a model that can call tools.

## The gap

Many agent CLIs focus on the conversation loop and leave authorization,
provenance, approvals, and recovery to the surrounding shell. Pandora keeps
those concerns in the runtime:

- Parliament supplies policy decisions.
- Shadow Council selects approved Harnesses, Genes, and providers.
- Harnesses compose domain behavior without becoming an authority.
- Genes request bounded work.
- ReferenceMonitor issues scoped, one-shot effect permits.
- EffectExecutor performs permitted effects and records receipts.

```mermaid
flowchart LR
    TUI["Pandora TUI / CLI"] --> EC["ExecutionController"]
    EC --> DSR["DSR Reasoning & Reflex<br/>(Signatures · CoT · S1 Logits · S2 LATS)"]
    DSR --> SC["Shadow Council<br/>(Selects Composition)"]
    SC --> PAR["Parliament<br/>(Policy & Approvals)"]
    PAR --> RM["ReferenceMonitor<br/>(One-Shot Permit)"]
    RM --> EX["EffectExecutor<br/>(Confined Sandbox)"]
    EX --> RCPT["Cryptographic Receipt & 5-Tier Memory"]
```

Learning and evolution components can propose or evaluate changes, but they
cannot issue permits, change policy roots, or activate themselves.

## What is shipped

The **v3.0.0 — SUNLESS** release provides:

- **Interactive 4-View Ratatui TUI Workbench**: immediate startup via `pandora` with live Sparkline, turn budget gauges, fuzzy palette (`Ctrl+P`), and 42 CLI subcommands;
- **Declarative Structured Reasoning (DSRs) & GEPA Optimizer**: typed reasoning signatures (`CodeReviewer`, `ReActModule`, `SqlSynthesizer`), automatic Chain-of-Thought (`.into_chain_of_thought()`), and 3D Pareto frontier optimization (pass rate, token cost, step latency);
- **Autonomous Self-Improvement**: 6-level optimization ladder ($L_0 \to L_5$), Darwin whole-agent genomes, 4-point safety spine, and $\text{Meta}^n$ recursive emergent depth with fixed-point $\Omega$ convergence;
- **Product Requirement Prompts (PRP) Pipeline**: 4-level task hierarchy and 4 progressive validation gates (Syntax $\to$ Unit $\to$ Integration $\to$ Receipt Check) with automated snapshot rollback;
- **System 1 Reflex Logits & System 2 LATS MCTS**: sub-millisecond logprob classification with active-learning uncertainty harvesting and PUCT Monte Carlo tree search;
- **5-Tier Crystallized Memory & ContextVfs**: `pandora://` virtual filesystem across L0 working rules, L1 insights, L2 global facts, L3 SOPs, and L4 session archives with Ed25519 P2P capsule mesh sync;
- **Confined Execution & OS-level Sandboxing**: capability-gated process executor, wasmi fuel-metered WebAssembly Genes, and local governed MCP stdio;
- **Durable Infrastructure**: SQLite WAL state stores, process fencing, PID leases, and multi-agent Diamond orchestration swarms;
- **Supply Chain Security**: Cosign keyless signatures, in-toto attestations, SLSA Provenance, and SPDX SBOM.

Pandora does not treat a type as proof that a product feature is complete.
Release notes and tests define the supported scope. Remote
execution and marketplace distribution sit outside the shipped boundary.

## Who it is for

Pandora is for developers, researchers, and teams who need to inspect what an
agent selected, what it was allowed to do, and what result was recorded. The
same runtime can support simple local tasks and controlled workflows while
keeping policy outside the model and outside self-improvement loops.

## Evidence

- [Runtime inventory](INVENTORY.md)
- [Harness model](HARNESSES.md)
- [CLI reference](CLI.md)
- [Evaluation model](EVALUATION.md)
- [Release policy](../RELEASES.md)
- [Current changelog](../CHANGELOG.md)
- [Installation guide](INSTALL.md)
- [Repository CI](../.github/workflows/ci.yml)
- [Security policy](../SECURITY.md)
