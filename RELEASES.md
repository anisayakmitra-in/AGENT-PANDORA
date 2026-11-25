# Release policy

Pandora ships as a native Rust CLI and interactive TUI.

Pandora uses plain SemVer tags and release titles. Stable and prerelease
versions use the same tag format; prereleases use neutral suffixes such as
`v2.0.0-alpha.1`, `v2.0.0-beta.1`, and `v2.0.0-rc.1`.

Existing legacy preview tags remain immutable.

The older `v2.0.0-anubis.1`, `v2.0.0-anubis.2`, and `v2.0.0-anubis.3` tags are
archived previews from an earlier naming scheme. They remain available for
history and reproducibility, but they are not the active release line or
recommended install targets.

The canonical published prerelease tags for this line are `v2.0.0-alpha.1`
through `v2.0.0-alpha.6`, followed by `v2.0.0-beta.1` through `v2.0.0-beta.10`,
and the stable releases `v2.0.0`, `v3.0.0`, `v3.0.1`, `v3.0.2`, `v3.0.3`, `v3.0.4`, `v3.0.5`, `v3.0.6`, `v3.0.7`, `v3.0.8`, `v3.0.9`, `v3.0.10`, `v3.1.0`, `v3.1.1`, `v3.1.2`, `v3.1.3`, `v3.1.4`, `v3.1.5`, `v4.0.0`, `v4.0.1`, `v4.0.2`, `v4.0.3`, `v4.0.4`, `v4.0.5`, `v4.0.6`, and `v4.0.7`. The active release identity
is `v4.0.7`.

## Source-bound release scope

Every new tag is evaluated against the exact tracked `release-scope.json` at its
source commit. This is a release contract, not a mutable workflow input:

`cli-only` is the only scope. It publishes the native CLI, launcher, installers,
checksums, signature, SBOM, provenance, notes, and CLI lifecycle evidence.
The release gate asserts
`cli-only` explicitly rather than assuming it, and the policy rejects any
other value.

Release-candidate and stable use the same `cli-only` boundary as alpha and beta.
No release channel requires a platform signature: binaries are not
Authenticode- or codesign-signed and are not notarized.

## v4.0.7

This release delivers Phases 46–48 of the architectural roadmap:
- **Phase 46: Quantum Annealing Graph Partitioning & QUBO Job Scheduling** (`quantum_annealer.rs`, Theorems 110–112, transverse-field Ising Hamiltonian ground state mapping, polynomial spectral gap convergence $\Delta \ge 1/\text{poly}(N)$, $(1 + \epsilon)$ Pareto-optimal makespan bound, CLI `pandora swarm anneal|qubo-schedule`, REST `/api/swarm/anneal`).
- **Phase 47: Homomorphic Federated Model Distillation & Secure Aggregation** (`federated_homomorphic.rs`, Theorems 113–115, IND-CPA security preserved under additive aggregation, Byzantine gradient clipping with geometric median filter, zero mutual information leak $I(\theta_i; \text{Coordinator}) = 0$, CLI `pandora crypto fed-aggregate|fed-verify`, REST `/api/crypto/federated`).
- **Phase 48: Causal Counterfactual Policy Inference & Pearlian Do-Calculus Engine** (`causal_engine.rs`, Theorems 116–118, backdoor criterion intervention identifiability $P(Y \mid do(X = x))$, monotonic counterfactual abduction-action-prediction consistency, confounder invariance bound, CLI `pandora cognitive do-calculus|counterfactual`, REST `/api/cognitive/causal`).
- **Cockpit Web Micro-Server REST Endpoints**: Added `/api/swarm/anneal`, `/api/crypto/federated`, `/api/cognitive/causal`, and updated `/api/olympus/overview` (`architecture_version: "4.0.7"`, `codename: "OLYMPUS"`, `total_theorems_enforced: 118`).
- Enforces all **118 Mathematical Foundation Theorems** under `#![forbid(unsafe_code)]`.

## v4.0.6

This release delivers Phases 43–45 of the architectural roadmap:
- **Phase 43: Recursive Zero-Knowledge Rollups & Algebraic STARK Proofs** (`recursive_zk_rollup.rs`, Theorems 101–103, constant-size STARK proof aggregation, BabyBear prime field $\mathbb{F}_p$, 128-bit computational soundness, CLI `pandora crypto zk-rollup|zk-verify`, REST `/api/crypto/rollup`).
- **Phase 44: Autonomous Self-Healing Kernel & Dynamic Invariant Synthesis** (`self_healing_kernel.rs`, Theorems 104–106, semantic bisimulation invariance under automated patch synthesis, quiescent state lock zero-downtime hot-patching, CLI `pandora governance heal|invariant-check`, REST `/api/governance/healing`).
- **Phase 45: Neuromorphic Spiking Context VFS & Temporal Memory Decay** (`neuromorphic_vfs.rs`, Theorems 107–109, Leaky Integrate-and-Fire membrane potential dynamics, STDP synaptic plasticity, exponential decay retention fidelity $\ge 95\%$, CLI `pandora memory spike-train|temporal-decay`, REST `/api/memory/neuromorphic`).
- **Cockpit Web Micro-Server REST Endpoints**: Added `/api/crypto/rollup`, `/api/governance/healing`, `/api/memory/neuromorphic`, and updated `/api/olympus/overview` (`architecture_version: "4.0.6"`, `codename: "OLYMPUS"`, `total_theorems_enforced: 109`).
- Enforces all **109 Mathematical Foundation Theorems** under `#![forbid(unsafe_code)]`.

## v4.0.5

This release delivers **Centennial Completeness (Phases 40–42, Theorems 92–100)**:
- **Phase 40: Distributed Multi-Tenant Mesh Orchestration & Cross-Cluster Atomic Commitments** (`multi_tenant_mesh.rs`, Theorems 92–94, shard attestation vector digests, 2-phase cross-cluster atomic commit receipts, CLI `pandora swarm mesh|cross-commit`, REST `/api/swarm/mesh`).
- **Phase 41: Hyper-Dimensional Vector VFS & Holographic Cognitive Associative Memory** (`hyper_vfs.rs`, Theorems 95–97, bipolar hypervectors $\mathbb{H}^{1024}$, circular convolution binding $\mathbf{a} \circledast \mathbf{b}$, circular correlation unbinding $\mathbf{c} \odot \mathbf{a}$, superposition memory bundling, CLI `pandora memory hyper-project|holographic-recall`, REST `/api/memory/hyper`).
- **Phase 42: Formal Kernel Verification & Lean 4 / Coq Machine-Checked Proof Exports** (`formal_proof.rs`, Theorems 98–100, formal proof generator and verification engine for all 100 mathematical foundation theorems in Lean 4 and Coq dialects, CLI `pandora audit formal-verify`, REST `/api/audit/formal`).
- **Cockpit Web Micro-Server REST Endpoints**: Added `/api/swarm/mesh`, `/api/memory/hyper`, `/api/audit/formal`, and updated `/api/olympus/overview` (`codename: "AETHER"`, `total_theorems_enforced: 100`).
- Enforces all **100 Mathematical Foundation Theorems** under `#![forbid(unsafe_code)]`.

## v4.0.4

This release delivers the **OLYMPUS Micro-Kernel Architecture Upgrade**:
- **Subsystem 1: Cognitive Micro-Kernel & Neuro-Symbolic Engine** (`cognitive.rs`, BNF pushdown automata token masking, LATS MCTS search, speculative tree rejection sampling, metacognitive entropy supervision, CLI `pandora cognitive status|speculate|synthesize|audit`).
- **Subsystem 2: Tripartite Constitutional Governance & Capability Bus** (`governance.rs`, capability-based expiring effect permits, declarative vetoes, process containment with <=50ms kill bound, CLI `pandora governance status|audit`).
- **Subsystem 3: Cryptographic Zero-Knowledge & Post-Quantum Substrate** (`crypto.rs`, post-quantum hybrid attestation with ML-DSA-65 + Ed25519, Goldilocks Poseidon sponge, Shamir Beaver triple MPC, CLI `pandora crypto status|pq-sign|mpc-eval`).
- **Subsystem 4: Decentralized Swarm Mesh & Consensus Substrate** (`swarm.rs`, PBFT consensus, conflict-free replicated data types, epidemic gossip dissemination).
- **Subsystem 5: Continual Memory VFS & Certified Privacy Substrate** (`memory.rs`, Context VFS PVFS v3, tropical HNSW indexing, Rényi DP accounting, certified unlearning).
- **Cockpit Web Micro-Server REST Endpoints**: `/api/subsystems/cognitive`, `/api/subsystems/governance`, `/api/subsystems/crypto`, `/api/subsystems/swarm`, `/api/subsystems/memory`, and `/api/olympus/overview`.
- Enforces all **91 Mathematical Foundation Theorems** under `#![forbid(unsafe_code)]`.

## v4.0.3

This release delivers Phases 36–38 of the architectural roadmap:
- **Phase 36**: Homomorphic Multi-Party Computation & Zero-Leakage Secret Sharing (Theorems 83–85, Shamir (k, n)-threshold sharing, Byzantine fault-tolerant reconstruction, Beaver triple homomorphic multiplication, CLI `pandora swarm secret-share|mpc-eval`, REST `/api/swarm/mpc`).
- **Phase 37**: Differential Privacy & Continual Certified Unlearning (Theorems 86–88, Rényi DP composition, projected Newton-step Fisher inverse scrubbing, Lipschitz memory stability, CLI `pandora memory scrub`, REST `/api/memory/privacy`).
- **Phase 38**: Distributed Speculative Multi-Draft Verification & Tree-Structured Decoding (Theorems 89–91, rejection sampling distributional equivalence, optimal tree draft speedup >= 2.0x, amortized single forward pass latency, CLI `pandora code speculate`, REST `/api/code/speculative`).

## v4.0.2

This release delivers Phases 33–35 of the architectural roadmap:
- **Phase 33**: Autonomous Untrusted Sandbox Isolation & Resource Roofline Hardening (Theorems 74–76, `SandboxIsolationEngine`, process tree kill bound <= 50ms, CLI `pandora sandbox status|execute|audit`, REST `/api/sandbox/telemetry`).
- **Phase 34**: Post-Quantum Hybrid Attestation & Algebraic Sponge State Commitments (Theorems 77–79, Poseidon Sponge over Goldilocks prime, dual classical Ed25519 + lattice ML-DSA-65 signatures, CLI `pandora moira pq-sign|pq-verify`, REST `/api/moira/pq`).
- **Phase 35**: Continuous Introspective Metacognition & Hallucination Entropy Gating (Theorems 80–82, predictive token entropy gating, monotonic backtracking convergence, PAC-Bayes generalization bound <= 0.15, CLI `pandora agent introspect|reflect`, REST `/api/agent/introspect`).

## v4.0.1

This release delivers Phases 31–32 of the architectural roadmap:
- **Phase 31**: Grammar-Constrained Token-Level BNF / Pushdown Automata Synthesis Engine (Theorems 68–70, zero parse errors by construction, CLI `pandora code synthesize`, `pandora code grammar mask|validate`, REST `/api/code/synthesize/grammar`).
- **Phase 32**: Decentralized Cross-Cluster Swarm CRDT & Epidemic Gossip Synchronization (Theorems 71–73, PN-Counter, OR-Set, Merkle DAG CRDT, Vector Clock causal precedence, CLI `pandora swarm crdt status|merge`, `pandora swarm gossip simulate`, REST `/api/swarm/crdt`).

## v4.0.0

Codename: **THANATOS**

Major workspace release delivering:
- **Phase 25**: Zero-Knowledge Proofs, Polynomial Attestation & Merkle Rollup (Theorems 48–49: Schwartz-Zippel Lemma, Reed-Solomon polynomial IOP verification, non-interactive batch attestation).
- **Phase 26**: Colab Remote GPU Bridge, VRAM Telemetry & 12 Specialized Inference Engines (Theorems 50–55: vLLM, TensorRT-LLM, SGLang, TGI, Ollama, Aphrodite, LMDeploy, DeepSpeed, Triton, MLC-LLM, ExLlamaV2, Candle).
- **Phase 27**: End-to-End Autonomous Multi-Agent Swarm SDLC Pipeline, Token-Level Grammar AST Patching & Pareto Frontier Optimization (Theorems 33, 35, 41, 56, 57: PAC-Bayes risk verification, pushdown automaton grammar patching, hierarchical KV-cache offloading).
- **Phase 28**: Unified Source, Domain & Meta Harness Tier Orchestration, Constitutional Substrates, and Genomic PAC-Bayes Evolutionary Adaptation Engine (Theorems 58–61: Cross-Domain Invariant Preservation, Recursive Meta-Synthesis, Byzantine Quorum Stability, Multi-Locus Generalization Convergence).
- Full `#![forbid(unsafe_code)]` compliance across all 7 workspace crates.

## v3.1.5

This release delivers Phases 9–12 of the architectural roadmap (autonomous in-loop `.pnn` hot-reload via `LatsLoopConfig::with_auto_calibrate`, quantized 7D `LatsPrefixMemoCache` memoization, tier-weighted tropical HNSW tie-breaking across `L0`–`L4`, and offline `verify-attestation` verification across CLI, TypeScript SDK, and C11/C++20 bindings under `#![forbid(unsafe_code)]`), along with the `url 2.5.9` security update (`RUSTSEC-2026-0592`).

## v3.1.4

This release delivers Phases 1–8 of the post-`v3.1.3` architectural roadmap: live per-turn System 2 LATS MCTS branching inside `AgentLoop` (`LatsLoopConfig`, `LatsTurnTelemetry`), persistent `PVFS` v3 binary disk indices (`VFS_INDEX_VERSION_V3 = 3`, dual 132B `QuantizedVector` + 512B `TropicalVector` tables with v1/v2 fallback) and default tropical HNSW construction with projective Hilbert dispersion tie-breaking (`active_hilbert_distance`), closed-loop reflex calibration (`pandora model|moira calibrate`, `calibrate_network_from_pairs`, `BayesianCanaryGate`), online System-2 -> System-1 `DpoPair` distillation inside `AgentLoop` (with `.pandora/dpo_pairs.jsonl` persistence), 7D `DecisionFeatures` multi-step feature-transition rollouts ($T: [0,1]^7 \times \{0,1,2,3\} \to [0,1]^7$) scored by calibrated `.pnn` networks, hash-chained `.pnn` calibration attestation receipts (`CalibrationAttestationReceipt`), and TypeScript SDK / C11 / C++20 ABI parity under `#![forbid(unsafe_code)]`.

## v3.1.3

This release connects `TropicalDecisionDriver` / `MoiraDecisionDriver` directly into System 2 LATS Monte Carlo Tree Search with PUCT selection, integrates fast-path reflex tool safety gating with entropy/margin escalation (`normalized_entropy > 0.666`) into `Parliament` and `ReferenceMonitor`, exposes `--strategy lats` (`--lats-rollouts`, `--lats-depth`, `--puct-c`, `--escalation-threshold`) on `pandora run`, introduces zero-multiplication max-plus `TropicalVector` semantic projections and hybrid BM25 + tropical ranking across `semantic_indexer.rs`, `hnsw.rs` (`HnswVectorType::Tropical = 4`), and `context_vfs.rs` (L0–L4 and `pandora memory vfs tropical-search`), and updates `scripts/release_predecessor.py` to treat `v3.1.0` and `v3.1.1` as unpublished so `v3.1.2` is selected as the published predecessor under `#![forbid(unsafe_code)]`.

## v3.1.2

This release wires `TropicalComplexityPrior` into `BanditModelRouter` and `ContextualThompsonSamplingRouter` for dynamic AST complexity-driven arm selection ($\tau_{\mathrm{adjusted}} = \mathrm{clamp}(0.50 + 0.25 \times (C - 0.50), 0.20, 0.85)$) via `.pandora/model_tpan.pnn`, adds the `pandora model` and `pandora moira` CLI subcommands (`inspect`, `doctor`/`status`, and explicit offline-first `sync` from `anisayakmitra-in/moira` on Hugging Face), and introduces deterministic `DecisionBenchmarkSuite` harnesses in `pandora-harnesses` comparing `MoiraDecisionDriver` and `HeuristicDecisionDriver` on real task traces under `#![forbid(unsafe_code)]`.

## v3.1.1

This patch release auto-activates `MoiraDecisionDriver::from_default_pandora_path()` in `SinglePassLogitEngine` with transparent fallback to `HeuristicDecisionDriver`, adds the pure-Rust `extract_ast_features` and `score_source` helpers for 16-dimensional structural AST scoring, and resolves Cargo workspace package metadata inheritance warnings.

## v3.1.0

This release introduces Phase 26 Tropical Polyhedral AST Networks (TPAN) with zero-multiplication inference, integrates `MoiraDecisionDriver` with 3-6-9 Dynamic Subspace Routing (DSR) entropy gating, and relicenses the repository under `GPL-3.0-or-later`.

## v3.0.10

This release implements Phase 23 of the mathematical runtime and governance roadmap: the Federated Policy Engine with reputation-weighted coordinate-wise trimmed mean gradient aggregation (Theorem 45) and dynamic Beta-conjugate Thompson sampling reputation with automated quarantine (Theorem 46) documented in `docs/math/phase23_federated_policy_and_reputation.md`, the Live Interactive Multi-Agent Autopilot Session orchestrator (`pandora cockpit autopilot`, `/autopilot`, and TUI `[A]`), and the Web / Mobile Remote Telemetry Dashboard micro-server (`pandora cockpit web --port 8080 --bind 0.0.0.0`) delivering real-time SSE telemetry streams and mobile/desktop responsive companion control under strict `#![forbid(unsafe_code)]`.

## v3.0.9

This release implements Phase 22 of the mathematical runtime and governance roadmap: the Interactive Ratatui Mission-Control Cockpit TUI (dynamically switchable 9-panel and 12-panel matrix layouts, keyboard navigation, drill-down telemetry modal, and headless snapshot fallbacks), the Swarm Byzantine Code Consensus Engine (coordinate-wise trimmed mean centroid and Krum outlier filtering over 4D metric quality space under Theorem 43), and the Pexer Multiplexer IPC Automation Engine (automated grid tiling, dry-run validation, and cross-platform script synthesis under Theorem 44) documented in `docs/math/phase22_cockpit_tui_and_consensus.md`.

## v3.0.8

This release implements Phase 21 of the mathematical runtime and governance roadmap: the 9/12-Panel Mission-Control Cockpit monitoring all runtime settings, harnesses, genes, and mathematical engines across Phases 1–20, and the Parallel Multi-Agent Swarm Collaboration Orchestrator supporting external coding agents (Antigravity, Claude Code, Codex, Gemini, Cursor, Devin, Cline, Hermes, Qwen) multiplexed via `pexer.exe` CLI/IPC or headless fallbacks, backed by Theorem 41 (Multi-Agent Byzantine Consensus Quorum) and Theorem 42 (Bounded Orchestration Latency and Permission Convergence) in `docs/math/cockpit_multi_agent_collaboration.md`.

## v3.0.7

This release implements Phases 18 through 20 of the mathematical runtime and governance roadmap (Discrete McKean-Vlasov Mean-Field Game equilibrium and Price-of-Anarchy bounding, McAllester/Maurer Empirical-Bernstein PAC-Bayesian generalization risk certificates and Catoni Gibbs posterior policy promotion gating, and Max-Plus semiring tropical DAG Kleene star makespan and Karp maximum cycle mean SLA throughput calculus) along with the expanded 12-Engine Unified Assurance Receipt pipeline wired directly into the live `GatewayController`, autonomous self-healing daemon guard loop (`pandora runtime daemon-guard`), and append-only hash-chained audit ledger checkpoints verified by `pandora audit verify` backed by 40 formal theorems across 20 phases in `docs/math/`.

## v3.0.6

This release implements Phases 15 through 17 of the mathematical runtime and governance roadmap (Split and Adaptive Conformal Prediction uncertainty calibration, Spectral Graph Laplacian Fiedler algebraic connectivity and Cheeger conductance certification, and Log-Domain Exp3 adversarial multi-armed bandit routing) along with the Unified Mathematical Assurance Receipt pipeline (`pandora runtime conformal|topology|bandit|assurance`) backed by 34 formal theorems across 17 phases in `docs/math/`.

## v3.0.5

This release completes CLI subcommand wiring across Phases 6 through 14 (`pandora models route`, `pandora audit`, `pandora privacy`, `pandora zk`, `pandora runtime speculative|kv-cache|scheduler|byzantine|drift|canary-gate`, and `pandora hardware scheduler`) and implements the next-generation mathematical runtime engines for Phases 9 through 14 (Rényi Differential Privacy accountant, Goldilocks-field Zero-Knowledge succinct state attestation, Blumofe-Leiserson adaptive work-stealing scheduler, Byzantine-resilient Multi-Krum & trimmed-mean consensus aggregator, log-domain Sinkhorn-Knopp entropic optimal transport semantic drift detector, and Ville martingale anytime-valid canary gate) backed by 28 formal theorems in `docs/math/`.

## v3.0.4

This release introduces direct CLI command and daemon endpoint wiring across Phases 2 through 5 (WebSocket RFC 6455 GET `/v1/ws` streaming daemon endpoint, epidemic gossip rounds and Bloom filter routing under `pandora peer gossip`, BFT quorum checks and DAG scheduling under `pandora fleet quorum`, Lyapunov-damped force-directed memory graph inspection under `pandora graph visual` and TUI Memory tab, and Roofline batch similarity and hardware benchmark telemetry under `pandora hardware`), alongside next-generation mathematical roadmap implementations (Phase 6 Contextual Thompson Sampling and UCB1 Multi-Armed Bandit model router with proven $O(\sqrt{K T \ln T})$ regret bounds, Phase 7 Merkle-DAG cryptographic audit log and verifiable state proofs with proven $O(\log N)$ inclusion proofs, and Phase 8 speculative token decoding and Radix Trie KV cache prefix compression).

## v3.0.3

This release introduces pure ARM64 NEON vector assembly dot-product acceleration (`fmla.4s` 4x unrolled) matching AVX2 throughput on Apple Silicon and Linux AArch64, scalar Int8 dense vector quantization (`QuantizedVector`) shrinking `ContextVfs` index storage by 75% while maintaining >99% cosine similarity ranking precision, SHA-256 incremental buffer diff caching, RFC 2104 compliant HMAC-SHA256 gateway security verification, and complete formal mathematical justifications.

## v3.0.2

This release introduces persistent disk serialization for the semantic vector index (`.pandora/indices/vfs.idx`), a live external messaging gateway service with HTTP webhook and bidirectional TCP socket event streaming (`pandora_bridge.py`), a background LSP diagnostics daemon streaming real-time compilation indicators into the TUI header, and AVX2 SIMD dot product acceleration with scalar fallback.

## v3.0.1

This patch release adds external messaging gateway integration (Telegram long-polling
daemon and Discord webhook dispatching with autonomous cron monitoring) and comprehensive
mouse and keyboard ergonomics to the Pandora interactive TUI workbench (horizontal scroll
tab switching, skills hub modal click-to-toggle, quick-action top bar buttons, direct approval
shortcuts, and readline word editing).

## v3.0.0

This is the major stable release of Pandora v3.0.0-SUNLESS. It updates the coupled Rust,
npm, installer, documentation, and SDK runtime surfaces to `3.0.0` while
retaining the source-bound `cli-only` release scope. It bundles Declarative Structured
Reasoning (DSRs), the Genetic-Pareto (GEPA) Optimizer, the 6-Level Optimization Ladder
($L_0 \to L_5$), Darwin Whole-Agent Genomes with task routing cards, Meta^n Recursive Emergent
Depth with fixed-point $\Omega$ convergence detection, Product Requirement Prompts (PRP)
progressive pipelines with 4-gate verification, and Universal Evolutionary Lineage tracing.

## v2.0.0

This is the first stable release of Pandora v2. It updates the coupled Rust,
npm, installer, documentation, and SDK runtime surfaces to `2.0.0` while
retaining the source-bound `cli-only` release scope. It bundles custom model
provider management, OAuth 2.0 PKCE loopback authentication with multi-account
switching, an interactive spotlight model/provider picker, a TrueColor terminal
mural backdrop, interactive workbench TUI startup, OS-level process confinement,
filesystem TOCTOU hardening, active policy lifecycle hooks, recursive self-improvement
governance, and dependency security updates.

## v2.0.0-beta.10

This is the next coherent prerelease identity. It updates the coupled Rust,
npm, installer, documentation, and SDK runtime surfaces to `2.0.0-beta.10` while
retaining the source-bound `cli-only` release scope. It bundles OS-level process
confinement, filesystem TOCTOU hardening, active policy hook enforcement, the
recursive self-improvement search and governance substrate, and dependency
security updates.

## v2.0.0-beta.9

This is the next coherent prerelease identity. It updates the coupled Rust,
npm, installer, documentation, and SDK runtime surfaces to `2.0.0-beta.9` while
retaining the source-bound `cli-only` release scope. It bundles durable Kanban
coordination, inter-agent society bot mode, session auto-prune and SQLite vacuum
compaction, agent-to-agent JSON-RPC 2.0 protocol v1.0, and browser PKCE OAuth 2.1
SaaS connectors.

## v2.0.0-beta.8

This is the next coherent prerelease identity. It updates the coupled Rust,
npm, desktop, installer, documentation, and SDK runtime surfaces to
`2.0.0-beta.8` while retaining the source-bound `cli-only` release scope. The
identity is prepared for review and is not a public release until its exact tag
workflow completes. The same CLI-only workflow is used for beta, RC, and stable.

## v2.0.0-beta.7

This beta groups bounded self-healing feedback, composition provenance,
verified memory synthesis, persisted graph snapshots, holdout evaluation, and
evidence-only evolution proposal intake. It is a CLI-only prerelease, not a
stable distribution claim.
The release workflow publishes release-evidence.json, linking checksums,
signatures, SBOM, provenance subjects, and platform artifacts.

## v2.0.0-beta.6

This beta adopts MIT licensing for Pandora-owned material, adds the native
Gemini provider adapter, hardens process-tree cleanup for verification and
MCP operations, adds durable evolution records and read-only evolution
inspection, and documents the governed Security Domain workflow. It is a
CLI-only prerelease; no desktop, remote-execution, or marketplace support is
claimed.

## v2.0.0-beta.5

This beta adds the read-only Data Domain Harness and completes the bounded
Security Domain workflow surface with a standard assessment entry point,
deep-scan evidence, and changed-code evidence. It remains a CLI-only
prerelease; these workflows do not execute scanners, modify code, or claim
security certification.

## v2.0.0-beta.4

This beta consolidates the read-only Security and Debugging Domain Harnesses.
It is a CLI-only prerelease with bounded evidence workflows, cross-platform
release verification, and no desktop or remote-execution support claim.

## v2.0.0-beta.3

This beta packages the governed Security Domain Harness, typed TypeScript CLI
client, and the current local MCP and runtime authority improvements. It is a
CLI-only prerelease; the built-in security workflows provide bounded evidence
and do not claim complete vulnerability scanning or automated remediation.

## v2.0.0-beta.2

This beta advances the CLI release line with durable context assembly caching,
bounded graph intelligence, deterministic golden-set evaluation, and the local
Fleet control plane. It also exposes those capabilities through stable JSON
commands for graph projections, evaluation, and Fleet operations. Runtime
authority remains unchanged: these projections and leases cannot issue effect
permits or bypass the ReferenceMonitor.

## v2.0.0-beta.1

This is the current CLI-only beta. It consolidates the governed local runtime,
authenticated loopback service, durable jobs, isolated subagents, local MCP,
signed package admission, bounded research evolution, release lifecycle tests,
and stable JSON automation contract recorded in [CHANGELOG.md](CHANGELOG.md).
Every effect remains bound to Parliament policy, a scoped execution profile,
the ReferenceMonitor, and a one-shot permit.

## v2.0.0-alpha.6

This was the final alpha prerelease. Enabled Skills contribute bounded
reference guidance to agent context. Disabled and suspended Skills are omitted,
oversized guidance fails closed, and Skill text cannot authorize effects,
change policy, satisfy approvals, or execute scripts.

## v2.0.0-alpha.1

This was the first CLI-only prerelease. Its shipped scope is recorded in
[CHANGELOG.md](CHANGELOG.md).

Historical preview tags are retained as immutable compatibility references and
are not the active release line.

It provides the CLI-first foundation:

- Parliament, Shadow Council, Domain Harness, Gene, and governed execution contracts.
- One-shot effect permits, receipts, policy decisions, and workspace-scoped executors.
- A coding Domain Harness with bounded read, search, patch, verify, and review operations.
- Provider manifests, bounded model requests, OpenAI-compatible HTTP transport, tool-call validation, and structured-output repair.
- Windows, macOS, and Linux CI, repository validation, dependency auditing, and release-build checks.

The preview is not a stable release and does not claim desktop support. Native
release artifacts are valid only when the tagged release workflow has
completed and its checksums, signature, SBOM, and provenance assets are
present.

## v2.0.0-alpha.2

This was a CLI-only prerelease. It carried the provider, session,
configuration, launcher, and CI improvements listed in
[CHANGELOG.md](CHANGELOG.md). The public command name remains `pandora`, and
the npm/Bun package name is `pandora-agent`.

The release remains a prerelease and does not claim desktop, mobile, remote,
or marketplace support. Native artifacts are valid only when the tagged
release workflow has completed and its checksums, signature, SBOM, and
provenance assets are present.

## v2.0.0-alpha.3

This CLI-only prerelease added the built-in `core-source` Source Harness,
session inspection, and completion support for session subcommands.

## v2.0.0-alpha.5

This was a previous CLI-only prerelease. It added local Skill package admission
through `pandora skill install`; admitted Skills remain disabled until an
operator enables them. The existing terminal clients and governed execution
path are unchanged.

## v2.0.0-alpha.4

This was a previous CLI-only prerelease. It added the line-oriented `pandora
chat` client and full-screen `pandora tui` client. Both reuse the existing
AgentLoop, session, approval, and governed effect path.

## Version rules

- Patch releases (`v2.0.x`) fix regressions without changing the public contract.
- Minor releases (`v2.x.0`) add compatible capabilities within the current release line.
- Stable major releases change the public contract and increment the major version.
- Prereleases use `-alpha.N`, `-beta.N`, or `-rc.N` suffixes.
- Release tags are immutable. Each release must publish notes, checksums, a signature, an SBOM, a release evidence index, supported-platform results, and rollback instructions.
- A release is not considered stable until local and GitHub checks pass on Windows, macOS, and Linux.
- Release-candidate and stable publication must pass the protected
  `release-publication` environment. Only `v*` tags may deploy through it, and a
  configured human reviewer must approve the publication job.
- A prerelease must not be described as a release candidate or as stable. Alpha
  and beta tags are prereleases; only `v*` tags enter the protected publication
  environment.
- No release channel requires a platform signature. Windows Authenticode, Apple
  Developer ID signing, and notarization were removed from the release process
  on 2026-10-05: Pandora is a terminal CLI distributed through channels that do
  not need an OS signature. Release binaries are therefore **not** OS-signed.
  Integrity is established instead by `checksums.txt`, its keyless cosign
  signature, and GitHub build attestations, all of which are verified in the
  release workflow and by the installers.

## Stable release approval

Alpha and beta tags are marked as prereleases and never enter the protected
publication environment. A release-candidate tag enters it only when
`PANDORA_RELEASE_CANDIDATE_APPROVED` is exactly `1`; a plain SemVer stable tag
likewise requires `PANDORA_STABLE_RELEASE_APPROVED` to be exactly `1`.

No release credential is required to publish. There is no certificate, no
signing identity, and no notarization account. The `release-publication`
environment is an approval gate and nothing else, so there is no secret that
could be leaked, logged, or committed by a signing job.

The published-package smoke job re-downloads checksum-bound artifacts,
authenticates the manifest and the binary with cosign, verifies the GitHub
build attestation, and exercises the CLI install, update, rollback, backup,
restore, and uninstall lifecycle. Current evidence therefore covers native
lifecycle and manifest integrity. It does not cover platform-signature or
notarization evidence, because no release artifact claims one.

## Stable rollback closure

The stable release index never claims rollback closure before published
lifecycle jobs run. After every stable publication, the release workflow waits
for all CLI install/update/backup/restore/rollback/uninstall jobs and
then emits `stable-rollback-evidence.json` bound to the tag, commit, predecessor,
and workflow run.

The first stable release in a compatible line has no older stable artifact. Its
evidence therefore records `pending_first_patch`, not success. For the 2.0 line,
the stable-to-stable gate can close only when `v2.0.1` legitimately rolls back
to the published `v2.0.0` artifacts. A patch tag without a compatible stable
predecessor fails closed.

All channels continue to require the signed checksum manifest, SPDX SBOM,
GitHub provenance, release notes, cross-platform tests, and published
install/update/rollback smoke tests described in
[production readiness](docs/PRODUCTION.md).
